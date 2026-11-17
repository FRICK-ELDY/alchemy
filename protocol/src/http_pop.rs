//! HTTP Proof-of-Possession for world host APIs (x0081 / x00d4).
//!
//! Guarantees: Ed25519 over `alchemy/pop/v1|{METHOD}|{path}|{ts}|{nonce}|{raw_body}`
//! bound to Room Token `cnf.spk`; callers enforce clock skew and single-use nonce.
//! Non-guarantees: RFC 9449 DPoP JWT shape; PlaceRead / auth-JWT routes; body streaming;
//! persisting nonce consume store across world restart.

use base64::Engine;
use ed25519_dalek::{Signature, Signer, Verifier, VerifyingKey};
use rand::rngs::OsRng;
use rand::RngCore;

use crate::wire_crypto::{
    public_key_bytes, SigningKey, WireCryptoError, PUBLIC_KEY_LENGTH, SIGNATURE_LENGTH,
};

/// Default accepted skew for `x-alchemy-pop-ts` (seconds).
pub const POP_TS_SKEW_SECS: u64 = 60;

/// Header name: Unix seconds (decimal).
pub const POP_TS_HEADER: &str = "x-alchemy-pop-ts";
/// Header name: single-use nonce (x00d4; base64url / opaque string, max 128 bytes).
pub const POP_NONCE_HEADER: &str = "x-alchemy-pop-nonce";
/// Header name: base64url (no pad) Ed25519 signature.
pub const POP_SIG_HEADER: &str = "x-alchemy-pop-sig";

/// Canonical message bytes signed by `cnf.spk` secret.
pub fn pop_signing_bytes(method: &str, path: &str, ts: u64, nonce: &str, body: &[u8]) -> Vec<u8> {
    let mut msg = format!("alchemy/pop/v1|{method}|{path}|{ts}|{nonce}|").into_bytes();
    msg.extend_from_slice(body);
    msg
}

/// Sign a host HTTP request; returns raw 64-byte signature.
pub fn sign_pop(
    key: &SigningKey,
    method: &str,
    path: &str,
    ts: u64,
    nonce: &str,
    body: &[u8],
) -> [u8; SIGNATURE_LENGTH] {
    let msg = pop_signing_bytes(method, path, ts, nonce, body);
    key.sign(&msg).to_bytes()
}

/// Verify PoP against a 32-byte `cnf.spk` public key.
pub fn verify_pop(
    session_pub: &[u8],
    method: &str,
    path: &str,
    ts: u64,
    nonce: &str,
    body: &[u8],
    sig: &[u8],
) -> Result<(), WireCryptoError> {
    if session_pub.len() != PUBLIC_KEY_LENGTH {
        return Err(WireCryptoError::BadPublicKeyLen(session_pub.len()));
    }
    if sig.len() != SIGNATURE_LENGTH {
        return Err(WireCryptoError::BadSignatureLen(sig.len()));
    }
    let arr: [u8; PUBLIC_KEY_LENGTH] = session_pub
        .try_into()
        .map_err(|_| WireCryptoError::BadPublicKeyLen(session_pub.len()))?;
    let vk = VerifyingKey::from_bytes(&arr).map_err(|_| WireCryptoError::BadPublicKey)?;
    let sig_arr: [u8; SIGNATURE_LENGTH] = sig
        .try_into()
        .map_err(|_| WireCryptoError::BadSignatureLen(sig.len()))?;
    let signature = Signature::from_bytes(&sig_arr);
    let msg = pop_signing_bytes(method, path, ts, nonce, body);
    vk.verify(&msg, &signature)
        .map_err(|_| WireCryptoError::VerifyFailed)
}

/// True when `|now - ts| <= skew` (unsigned distance).
pub fn pop_ts_in_skew(now: u64, ts: u64, skew: u64) -> bool {
    now.abs_diff(ts) <= skew
}

/// Convenience: public key bytes for JWT `cnf.spk` / headers.
pub fn pop_session_pub(key: &SigningKey) -> Vec<u8> {
    public_key_bytes(key)
}

/// Fresh 16-byte PoP nonce as base64url (no pad) for `x-alchemy-pop-nonce`.
pub fn fresh_pop_nonce() -> String {
    let mut bytes = [0u8; 16];
    OsRng.fill_bytes(&mut bytes);
    base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(bytes)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::wire_crypto::generate_signing_key;

    #[test]
    fn pop_roundtrip() {
        let key = generate_signing_key();
        let pub_bytes = public_key_bytes(&key);
        let body = br#"{"host_pub":"abc"}"#;
        let ts = 1_700_000_000u64;
        let nonce = "n1";
        let sig = sign_pop(&key, "POST", "/api/v1/rooms/r/host_keys", ts, nonce, body);
        assert!(verify_pop(
            &pub_bytes,
            "POST",
            "/api/v1/rooms/r/host_keys",
            ts,
            nonce,
            body,
            &sig
        )
        .is_ok());
        assert!(verify_pop(
            &pub_bytes,
            "POST",
            "/api/v1/rooms/r/host_keys",
            ts,
            nonce,
            br#"{"host_pub":"evil"}"#,
            &sig
        )
        .is_err());
        assert!(verify_pop(
            &pub_bytes,
            "POST",
            "/api/v1/rooms/r/host_keys",
            ts,
            "other-nonce",
            body,
            &sig
        )
        .is_err());
    }

    #[test]
    fn skew_window() {
        assert!(pop_ts_in_skew(100, 100, 60));
        assert!(pop_ts_in_skew(100, 160, 60));
        assert!(!pop_ts_in_skew(100, 161, 60));
        assert!(pop_ts_in_skew(100, 40, 60));
        assert!(!pop_ts_in_skew(100, 39, 60));
    }
}
