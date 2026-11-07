//! Room-key AEAD for Zenoh payloads (x0069 / x007e epoch).
//!
//! # Guarantees
//! - XChaCha20Poly1305 with 32-byte room key and per-message 192-bit random nonce.
//! - Envelope: `nonce(24) || ciphertext||tag(16)`.
//! - AAD binds `room_id`, `room_key_epoch`, and wire kind so cross-room / cross-epoch /
//!   cross-kind ciphertext swap fails (x007e).
//! - Open failure → caller must discard before protobuf / signature checks.
//!
//! # Non-guarantees
//! - Topic-name (`wire_cap`) secrecy; put DoS; forward secrecy among concurrent joiners
//!   (shared key model). Previous-epoch grace is host/application policy (`open_current_or_prev`).

use base64::Engine;
use chacha20poly1305::aead::{Aead, KeyInit, Payload};
use chacha20poly1305::{XChaCha20Poly1305, XNonce};
use rand::rngs::OsRng;
use rand::RngCore;

use crate::decode_limits::WireClass;

/// Raw room key length (bytes).
pub const ROOM_KEY_LEN: usize = 32;
/// XChaCha20Poly1305 nonce length.
pub const AEAD_NONCE_LEN: usize = 24;
/// Poly1305 tag length (appended to ciphertext by the AEAD).
pub const AEAD_TAG_LEN: usize = 16;
/// `nonce || ct||tag` overhead beyond plaintext.
pub const AEAD_OVERHEAD: usize = AEAD_NONCE_LEN + AEAD_TAG_LEN;

/// AAD scheme version prefix (ASCII). x007e bumped v1 → v2 (epoch in AAD).
pub const AEAD_AAD_PREFIX: &str = "alchemy/aead/v2";

/// AEAD kind bound into AAD (includes snapshot, distinct from frame).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AeadKind {
    Manifest,
    Frame,
    Presence,
    Snapshot,
    Input,
    /// Low-frequency inventory take-out spawn (x006d); carries host Room Token.
    InventorySpawn,
    /// Low-frequency inventory scene despawn (x007b); carries host Room Token.
    InventoryDespawn,
}

impl AeadKind {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Manifest => "manifest",
            Self::Frame => "frame",
            Self::Presence => "presence",
            Self::Snapshot => "snapshot",
            Self::Input => "input",
            Self::InventorySpawn => "inventory_spawn",
            Self::InventoryDespawn => "inventory_despawn",
        }
    }

    /// Map to plaintext [`WireClass`] ceilings (snapshot shares Frame limits).
    pub const fn wire_class(self) -> WireClass {
        match self {
            Self::Manifest => WireClass::Manifest,
            Self::Frame | Self::Snapshot => WireClass::Frame,
            Self::Presence => WireClass::Presence,
            Self::Input | Self::InventorySpawn | Self::InventoryDespawn => WireClass::Input,
        }
    }
}

/// Why seal/open failed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WireAeadError {
    BadKeyLen(usize),
    CiphertextTooShort(usize),
    CryptographicFailure,
}

impl std::fmt::Display for WireAeadError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::BadKeyLen(n) => write!(f, "room_key must be {ROOM_KEY_LEN} bytes, got {n}"),
            Self::CiphertextTooShort(n) => {
                write!(f, "AEAD envelope too short: {n} < {AEAD_OVERHEAD}")
            }
            Self::CryptographicFailure => write!(f, "AEAD authenticate/decrypt failed"),
        }
    }
}

impl std::error::Error for WireAeadError {}

/// Build AAD: `alchemy/aead/v2|{room_id}|{epoch}|{kind}`.
pub fn aead_aad(room_id: &str, epoch: u64, kind: AeadKind) -> String {
    format!("{AEAD_AAD_PREFIX}|{room_id}|{epoch}|{}", kind.as_str())
}

fn cipher(key: &[u8]) -> Result<XChaCha20Poly1305, WireAeadError> {
    if key.len() != ROOM_KEY_LEN {
        return Err(WireAeadError::BadKeyLen(key.len()));
    }
    Ok(XChaCha20Poly1305::new(key.into()))
}

/// Encrypt `plaintext` → `nonce || ciphertext||tag`.
pub fn seal(
    room_key: &[u8],
    room_id: &str,
    epoch: u64,
    kind: AeadKind,
    plaintext: &[u8],
) -> Result<Vec<u8>, WireAeadError> {
    let cipher = cipher(room_key)?;
    let mut nonce_bytes = [0u8; AEAD_NONCE_LEN];
    OsRng.fill_bytes(&mut nonce_bytes);
    let nonce = XNonce::from_slice(&nonce_bytes);
    let aad = aead_aad(room_id, epoch, kind);
    let ct = cipher
        .encrypt(
            nonce,
            Payload {
                msg: plaintext,
                aad: aad.as_bytes(),
            },
        )
        .map_err(|_| WireAeadError::CryptographicFailure)?;
    let mut out = Vec::with_capacity(AEAD_NONCE_LEN + ct.len());
    out.extend_from_slice(&nonce_bytes);
    out.extend_from_slice(&ct);
    Ok(out)
}

/// Decrypt `nonce || ciphertext||tag` → plaintext. Fail-closed on auth error.
pub fn open(
    room_key: &[u8],
    room_id: &str,
    epoch: u64,
    kind: AeadKind,
    envelope: &[u8],
) -> Result<Vec<u8>, WireAeadError> {
    if envelope.len() < AEAD_OVERHEAD {
        return Err(WireAeadError::CiphertextTooShort(envelope.len()));
    }
    let cipher = cipher(room_key)?;
    let (nonce_bytes, ct) = envelope.split_at(AEAD_NONCE_LEN);
    let nonce = XNonce::from_slice(nonce_bytes);
    let aad = aead_aad(room_id, epoch, kind);
    cipher
        .decrypt(
            nonce,
            Payload {
                msg: ct,
                aad: aad.as_bytes(),
            },
        )
        .map_err(|_| WireAeadError::CryptographicFailure)
}

/// Try current key/epoch, then optional previous (x007e host grace).
pub fn open_current_or_prev(
    current_key: &[u8],
    current_epoch: u64,
    prev: Option<(&[u8], u64)>,
    room_id: &str,
    kind: AeadKind,
    envelope: &[u8],
) -> Result<Vec<u8>, WireAeadError> {
    match open(current_key, room_id, current_epoch, kind, envelope) {
        Ok(pt) => Ok(pt),
        Err(WireAeadError::CryptographicFailure) => {
            if let Some((prev_key, prev_epoch)) = prev {
                open(prev_key, room_id, prev_epoch, kind, envelope)
            } else {
                Err(WireAeadError::CryptographicFailure)
            }
        }
        Err(other) => Err(other),
    }
}

/// Ciphertext (envelope) byte ceiling for `kind` = plaintext max + overhead.
pub const fn max_aead_envelope_bytes(kind: AeadKind) -> usize {
    crate::decode_limits::max_payload_bytes(kind.wire_class()) + AEAD_OVERHEAD
}

/// Reject oversized AEAD envelopes before open.
pub fn check_aead_envelope_bytes(
    kind: AeadKind,
    len: usize,
) -> Result<(), crate::decode_limits::DecodeLimitError> {
    let class = kind.wire_class();
    let max = max_aead_envelope_bytes(kind);
    if len > max {
        Err(crate::decode_limits::DecodeLimitError::PayloadTooLarge { class, len, max })
    } else {
        Ok(())
    }
}

/// Decode URL-safe base64 (no pad) room_key from Token JSON → 32 raw bytes.
pub fn decode_room_key_b64(b64: &str) -> Result<[u8; ROOM_KEY_LEN], WireAeadError> {
    let raw = base64::engine::general_purpose::URL_SAFE_NO_PAD
        .decode(b64)
        .map_err(|_| WireAeadError::CryptographicFailure)?;
    if raw.len() != ROOM_KEY_LEN {
        return Err(WireAeadError::BadKeyLen(raw.len()));
    }
    let mut out = [0u8; ROOM_KEY_LEN];
    out.copy_from_slice(&raw);
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn key() -> [u8; ROOM_KEY_LEN] {
        [0x42; ROOM_KEY_LEN]
    }

    #[test]
    fn roundtrip_manifest() {
        let pt = b"hello-manifest";
        let env = seal(&key(), "local", 0, AeadKind::Manifest, pt).unwrap();
        assert!(env.len() >= AEAD_OVERHEAD + pt.len());
        let out = open(&key(), "local", 0, AeadKind::Manifest, &env).unwrap();
        assert_eq!(out, pt);
    }

    #[test]
    fn roundtrip_inventory_spawn() {
        let pt = br#"{"room_token":"t","blake3":"aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"}"#;
        let env = seal(&key(), "local", 1, AeadKind::InventorySpawn, pt).unwrap();
        let out = open(&key(), "local", 1, AeadKind::InventorySpawn, &env).unwrap();
        assert_eq!(out, pt);
        let env2 = seal(&key(), "local", 1, AeadKind::InventoryDespawn, pt).unwrap();
        let out2 = open(&key(), "local", 1, AeadKind::InventoryDespawn, &env2).unwrap();
        assert_eq!(out2, pt);
        assert!(open(&key(), "local", 1, AeadKind::InventoryDespawn, &env).is_err());
        assert_eq!(
            open(&key(), "local", 1, AeadKind::Input, &env),
            Err(WireAeadError::CryptographicFailure)
        );
    }

    #[test]
    fn wrong_room_id_fails() {
        let env = seal(&key(), "alpha", 0, AeadKind::Frame, b"pose").unwrap();
        assert_eq!(
            open(&key(), "beta", 0, AeadKind::Frame, &env),
            Err(WireAeadError::CryptographicFailure)
        );
    }

    #[test]
    fn wrong_kind_fails() {
        let env = seal(&key(), "local", 0, AeadKind::Manifest, b"m").unwrap();
        assert_eq!(
            open(&key(), "local", 0, AeadKind::Frame, &env),
            Err(WireAeadError::CryptographicFailure)
        );
    }

    #[test]
    fn wrong_epoch_fails() {
        let env = seal(&key(), "local", 3, AeadKind::Frame, b"pose").unwrap();
        assert_eq!(
            open(&key(), "local", 2, AeadKind::Frame, &env),
            Err(WireAeadError::CryptographicFailure)
        );
    }

    #[test]
    fn open_current_or_prev_accepts_prev() {
        let prev_key = [0x11u8; ROOM_KEY_LEN];
        let env = seal(&prev_key, "local", 4, AeadKind::Input, b"old").unwrap();
        let out = open_current_or_prev(
            &key(),
            5,
            Some((&prev_key, 4)),
            "local",
            AeadKind::Input,
            &env,
        )
        .unwrap();
        assert_eq!(out, b"old");
    }

    #[test]
    fn snapshot_not_frame_aad() {
        let env = seal(&key(), "local", 0, AeadKind::Snapshot, b"full").unwrap();
        assert_eq!(
            open(&key(), "local", 0, AeadKind::Frame, &env),
            Err(WireAeadError::CryptographicFailure)
        );
        assert_eq!(
            open(&key(), "local", 0, AeadKind::Snapshot, &env).unwrap(),
            b"full"
        );
    }

    #[test]
    fn tampered_tag_fails() {
        let mut env = seal(&key(), "local", 0, AeadKind::Input, b"intent").unwrap();
        let last = env.len() - 1;
        env[last] ^= 0xff;
        assert_eq!(
            open(&key(), "local", 0, AeadKind::Input, &env),
            Err(WireAeadError::CryptographicFailure)
        );
    }

    #[test]
    fn wrong_key_fails() {
        let env = seal(&key(), "local", 0, AeadKind::Presence, b"p").unwrap();
        let other = [0x11u8; ROOM_KEY_LEN];
        assert_eq!(
            open(&other, "local", 0, AeadKind::Presence, &env),
            Err(WireAeadError::CryptographicFailure)
        );
    }

    #[test]
    fn short_envelope_fails() {
        assert_eq!(
            open(&key(), "local", 0, AeadKind::Frame, &[0u8; 10]),
            Err(WireAeadError::CiphertextTooShort(10))
        );
    }

    #[test]
    fn room_key_b64_roundtrip() {
        let raw = key();
        let b64 = base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(raw);
        assert_eq!(decode_room_key_b64(&b64).unwrap(), raw);
    }
}
