//! Ed25519 session / host signatures for Intent, RenderFrame, Presence (x0023 / x0025).
//! Manifest seal body hashing for world ManifestSeal JWT (x0026).
//!
//! # Guarantees
//! - Sign / verify over prost encode with the signature field cleared.
//! - Reject wrong key lengths and invalid signatures.
//! - Manifest seal blake3 is over protobuf with `world_seal` cleared.
//!
//! # Non-guarantees
//! - Key distribution beyond world HostAttestation → Manifest `host_pub` / Intent `session_pub`.
//! - Forward secrecy, rotation ceremonies, hardware-backed keys.
//! - SceneManifest-driven content (P4).

use ed25519_dalek::{Signature, Signer, Verifier, VerifyingKey, SIGNATURE_LENGTH};
use prost::Message;
use rand::rngs::OsRng;

use crate::{InputEvent, PresenceState, RenderFrame, SceneManifest};

pub use ed25519_dalek::SigningKey;

pub const PUBLIC_KEY_LENGTH: usize = 32;

/// Generate a fresh Ed25519 signing key (OsRng).
pub fn generate_signing_key() -> SigningKey {
    SigningKey::generate(&mut OsRng)
}

/// 32-byte public key bytes for wire fields.
pub fn public_key_bytes(key: &SigningKey) -> Vec<u8> {
    key.verifying_key().to_bytes().to_vec()
}

fn verifying_key_from(bytes: &[u8]) -> Result<VerifyingKey, WireCryptoError> {
    if bytes.len() != PUBLIC_KEY_LENGTH {
        return Err(WireCryptoError::BadPublicKeyLen(bytes.len()));
    }
    let arr: [u8; PUBLIC_KEY_LENGTH] = bytes
        .try_into()
        .map_err(|_| WireCryptoError::BadPublicKeyLen(bytes.len()))?;
    VerifyingKey::from_bytes(&arr).map_err(|_| WireCryptoError::BadPublicKey)
}

fn signature_from(bytes: &[u8]) -> Result<Signature, WireCryptoError> {
    if bytes.len() != SIGNATURE_LENGTH {
        return Err(WireCryptoError::BadSignatureLen(bytes.len()));
    }
    let arr: [u8; SIGNATURE_LENGTH] = bytes
        .try_into()
        .map_err(|_| WireCryptoError::BadSignatureLen(bytes.len()))?;
    Ok(Signature::from_bytes(&arr))
}

/// Encode `InputEvent` for signing (`intent_sig` cleared).
pub fn input_signing_bytes(event: &InputEvent) -> Vec<u8> {
    let mut e = event.clone();
    e.intent_sig.clear();
    e.encode_to_vec()
}

/// Sign Intent in place (`intent_sig` filled).
pub fn sign_input(key: &SigningKey, event: &mut InputEvent) {
    event.intent_sig.clear();
    let msg = event.encode_to_vec();
    event.intent_sig = key.sign(&msg).to_bytes().to_vec();
}

/// Verify Intent signature with `session_pub`.
pub fn verify_input(event: &InputEvent) -> Result<(), WireCryptoError> {
    let vk = verifying_key_from(&event.session_pub)?;
    let sig = signature_from(&event.intent_sig)?;
    let msg = input_signing_bytes(event);
    vk.verify(&msg, &sig)
        .map_err(|_| WireCryptoError::VerifyFailed)
}

/// Encode `RenderFrame` for signing (`host_sig` cleared).
pub fn frame_signing_bytes(frame: &RenderFrame) -> Vec<u8> {
    let mut f = frame.clone();
    f.host_sig.clear();
    f.encode_to_vec()
}

/// Sign Frame in place (`host_sig` filled).
pub fn sign_frame(key: &SigningKey, frame: &mut RenderFrame) {
    frame.host_sig.clear();
    let msg = frame.encode_to_vec();
    frame.host_sig = key.sign(&msg).to_bytes().to_vec();
}

/// Verify Frame with Manifest `host_pub`.
pub fn verify_frame(host_pub: &[u8], frame: &RenderFrame) -> Result<(), WireCryptoError> {
    let vk = verifying_key_from(host_pub)?;
    let sig = signature_from(&frame.host_sig)?;
    let msg = frame_signing_bytes(frame);
    vk.verify(&msg, &sig)
        .map_err(|_| WireCryptoError::VerifyFailed)
}

/// Encode `PresenceState` for signing (`host_sig` cleared).
pub fn presence_signing_bytes(state: &PresenceState) -> Vec<u8> {
    let mut s = state.clone();
    s.host_sig.clear();
    s.encode_to_vec()
}

/// Sign Presence in place (`host_sig` filled).
pub fn sign_presence(key: &SigningKey, state: &mut PresenceState) {
    state.host_sig.clear();
    let msg = state.encode_to_vec();
    state.host_sig = key.sign(&msg).to_bytes().to_vec();
}

/// Verify Presence with attested Manifest `host_pub` (x0025).
pub fn verify_presence(host_pub: &[u8], state: &PresenceState) -> Result<(), WireCryptoError> {
    let vk = verifying_key_from(host_pub)?;
    let sig = signature_from(&state.host_sig)?;
    let msg = presence_signing_bytes(state);
    vk.verify(&msg, &sig)
        .map_err(|_| WireCryptoError::VerifyFailed)
}

/// Encode `SceneManifest` for world seal hashing (`world_seal` cleared).
pub fn manifest_seal_bytes(manifest: &SceneManifest) -> Vec<u8> {
    let mut m = manifest.clone();
    m.world_seal.clear();
    m.encode_to_vec()
}

/// Lowercase hex BLAKE3 of [`manifest_seal_bytes`] (x0026).
pub fn manifest_seal_blake3_hex(manifest: &SceneManifest) -> String {
    blake3::hash(&manifest_seal_bytes(manifest))
        .to_hex()
        .to_string()
}

/// True when all Entity numeric fields are finite and orientation is usable (x0029).
///
/// Rejects proto3 zero quat (invalid). Near-unit after normalize is not required here —
/// clients normalize; hosts should publish unit quats. Scale may be all-zero (unset → 1,1,1).
pub fn entity_is_finite(e: &crate::Entity) -> bool {
    e.x.is_finite()
        && e.y.is_finite()
        && e.z.is_finite()
        && e.r.is_finite()
        && e.g.is_finite()
        && e.b.is_finite()
        && e.a.is_finite()
        && e.qx.is_finite()
        && e.qy.is_finite()
        && e.qz.is_finite()
        && e.qw.is_finite()
        && e.sx.is_finite()
        && e.sy.is_finite()
        && e.sz.is_finite()
        && entity_quat_nonzero(e)
}

fn entity_quat_nonzero(e: &crate::Entity) -> bool {
    let n2 = e.qx * e.qx + e.qy * e.qy + e.qz * e.qz + e.qw * e.qw;
    n2.is_finite() && n2 > 1e-24
}

/// True when every upsert in the frame is finite.
pub fn frame_entities_finite(frame: &RenderFrame) -> bool {
    frame.upserts.iter().all(entity_is_finite)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WireCryptoError {
    BadPublicKeyLen(usize),
    BadPublicKey,
    BadSignatureLen(usize),
    VerifyFailed,
}

impl std::fmt::Display for WireCryptoError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::BadPublicKeyLen(n) => {
                write!(f, "public key length {n}, expected {PUBLIC_KEY_LENGTH}")
            }
            Self::BadPublicKey => write!(f, "invalid Ed25519 public key"),
            Self::BadSignatureLen(n) => {
                write!(f, "signature length {n}, expected {SIGNATURE_LENGTH}")
            }
            Self::VerifyFailed => write!(f, "Ed25519 verify failed"),
        }
    }
}

impl std::error::Error for WireCryptoError {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn input_roundtrip_sign_verify() {
        let key = generate_signing_key();
        let mut event = InputEvent {
            room_token: "jwt".into(),
            client_time_ms: 1,
            move_x: 0.1,
            move_y: 0.0,
            move_z: -0.2,
            buttons: 0,
            grab_entity_id: 0,
            aim_x: 0.0,
            aim_y: 1.0,
            aim_z: 0.0,
            leave: false,
            session_pub: public_key_bytes(&key),
            intent_seq: 1,
            intent_sig: vec![],
        };
        sign_input(&key, &mut event);
        assert!(verify_input(&event).is_ok());

        event.move_x = 0.9;
        assert!(verify_input(&event).is_err());
    }

    #[test]
    fn frame_roundtrip_sign_verify() {
        let key = generate_signing_key();
        let mut frame = RenderFrame {
            tick: 7,
            host_time_ms: 100,
            upserts: vec![crate::entity_at(1, 1.0, 2.0, 3.0, 1.0, 1.0, 1.0, 1.0)],
            removes: vec![],
            host_sig: vec![],
        };
        sign_frame(&key, &mut frame);
        assert!(verify_frame(&public_key_bytes(&key), &frame).is_ok());
        assert!(verify_frame(&[0u8; 32], &frame).is_err());
    }

    #[test]
    fn rejects_non_finite_entity() {
        let mut e = crate::entity_at(1, 0.0, 0.0, 0.0, 1.0, 1.0, 1.0, 1.0);
        assert!(entity_is_finite(&e));
        e.x = f64::NAN;
        assert!(!entity_is_finite(&e));
        e = crate::entity_at(1, 0.0, 0.0, 0.0, 1.0, 1.0, 1.0, 1.0);
        e.qw = f64::INFINITY;
        assert!(!entity_is_finite(&e));
        e = crate::entity_at(1, 0.0, 0.0, 0.0, 1.0, 1.0, 1.0, 1.0);
        e.sx = f64::NAN;
        assert!(!entity_is_finite(&e));
        e = crate::entity_at(1, 0.0, 0.0, 0.0, 1.0, 1.0, 1.0, 1.0);
        e.qx = 0.0;
        e.qy = 0.0;
        e.qz = 0.0;
        e.qw = 0.0;
        assert!(!entity_is_finite(&e));
    }

    #[test]
    fn presence_roundtrip_sign_verify() {
        let key = generate_signing_key();
        let mut state = PresenceState {
            revision: 3,
            assignments: vec![],
            protocol_pin: crate::PROTOCOL_PIN.into(),
            host_sig: vec![],
            host_generation: 1,
        };
        sign_presence(&key, &mut state);
        assert!(verify_presence(&public_key_bytes(&key), &state).is_ok());
        assert!(verify_presence(&[0u8; 32], &state).is_err());
    }
}
