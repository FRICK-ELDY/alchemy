//! Inventory control-plane session signatures (x0081).
//!
//! Guarantees: spawn/despawn Zenoh JSON carries `session_pub` + `control_seq` + Ed25519
//! `control_sig` over a canonical UTF-8 message (no Room Token on the bus).
//! Non-guarantees: SessionTable bind / owner checks (host-server); world HTTP PoP.

use ed25519_dalek::{Signature, Signer, Verifier, VerifyingKey};

use crate::wire_crypto::{
    WireCryptoError, SigningKey, PUBLIC_KEY_LENGTH, SIGNATURE_LENGTH,
};

/// Pose TRS for spawn control: x y z qx qy qz qw sx sy sz.
pub type ControlPose = [f64; 10];

/// Canonical spawn message (f64 via `to_bits` hex for stable wire).
pub fn spawn_signing_bytes(
    room_id: &str,
    control_seq: u64,
    blake3_hex: &str,
    pose: ControlPose,
) -> Vec<u8> {
    let mut s = format!("alchemy/control/v1|spawn|{room_id}|{control_seq}|{blake3_hex}");
    for v in pose {
        s.push('|');
        s.push_str(&format!("{:016x}", v.to_bits()));
    }
    s.into_bytes()
}

/// Canonical despawn message.
pub fn despawn_signing_bytes(room_id: &str, control_seq: u64, entity_id: u32) -> Vec<u8> {
    format!("alchemy/control/v1|despawn|{room_id}|{control_seq}|{entity_id}").into_bytes()
}

pub fn sign_spawn(
    key: &SigningKey,
    room_id: &str,
    control_seq: u64,
    blake3_hex: &str,
    pose: ControlPose,
) -> [u8; SIGNATURE_LENGTH] {
    let msg = spawn_signing_bytes(room_id, control_seq, blake3_hex, pose);
    key.sign(&msg).to_bytes()
}

pub fn verify_spawn(
    session_pub: &[u8],
    room_id: &str,
    control_seq: u64,
    blake3_hex: &str,
    pose: ControlPose,
    sig: &[u8],
) -> Result<(), WireCryptoError> {
    verify_msg(
        session_pub,
        &spawn_signing_bytes(room_id, control_seq, blake3_hex, pose),
        sig,
    )
}

pub fn sign_despawn(
    key: &SigningKey,
    room_id: &str,
    control_seq: u64,
    entity_id: u32,
) -> [u8; SIGNATURE_LENGTH] {
    let msg = despawn_signing_bytes(room_id, control_seq, entity_id);
    key.sign(&msg).to_bytes()
}

pub fn verify_despawn(
    session_pub: &[u8],
    room_id: &str,
    control_seq: u64,
    entity_id: u32,
    sig: &[u8],
) -> Result<(), WireCryptoError> {
    verify_msg(
        session_pub,
        &despawn_signing_bytes(room_id, control_seq, entity_id),
        sig,
    )
}

fn verify_msg(session_pub: &[u8], msg: &[u8], sig: &[u8]) -> Result<(), WireCryptoError> {
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
    vk.verify(msg, &signature)
        .map_err(|_| WireCryptoError::VerifyFailed)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::wire_crypto::{generate_signing_key, public_key_bytes};

    #[test]
    fn spawn_roundtrip_no_token_field() {
        let key = generate_signing_key();
        let pub_bytes = public_key_bytes(&key);
        let pose = [1.0, 1.5, 2.0, 0.0, 0.0, 0.0, 1.0, 1.0, 1.0, 1.0];
        let blake3 = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
        let sig = sign_spawn(&key, "local", 1, blake3, pose);
        assert!(verify_spawn(&pub_bytes, "local", 1, blake3, pose, &sig).is_ok());
        assert!(verify_spawn(&pub_bytes, "local", 2, blake3, pose, &sig).is_err());
        let msg = String::from_utf8(spawn_signing_bytes("local", 1, blake3, pose)).unwrap();
        assert!(!msg.contains("room_token"));
        assert!(msg.starts_with("alchemy/control/v1|spawn|"));
    }

    #[test]
    fn despawn_roundtrip() {
        let key = generate_signing_key();
        let pub_bytes = public_key_bytes(&key);
        let sig = sign_despawn(&key, "local", 3, 42);
        assert!(verify_despawn(&pub_bytes, "local", 3, 42, &sig).is_ok());
        assert!(verify_despawn(&pub_bytes, "local", 3, 43, &sig).is_err());
    }
}
