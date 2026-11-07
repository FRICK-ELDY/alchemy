//! Manifest trust pipeline (x0038 / x003a / x0069 / x0070).
//!
//! Guarantees: verify failures yield [`TrustOutcome::Discard`] only; Accept updates
//! [`TrustWatermarks`]. Non-guarantees: mesh download, snapshot GET, GUI leave.

use crate::gate::gate_manifest_identity;
use crate::host_attestation::HostAttestationVerifier;
use crate::manifest_seal::ManifestSealVerifier;
use alchemy_protocol::decode_limits::{check_manifest, check_payload_bytes, WireClass};
use alchemy_protocol::generation_gate::{decide_gen_rev, GenRev, GenRevDecision};
use alchemy_protocol::wire_aead::{self, AeadKind};
use alchemy_protocol::wire_crypto::manifest_seal_blake3_hex;
use alchemy_protocol::SceneManifest;
use prost::Message;

/// Joined-room watermarks mutated only on [`TrustOutcome::Accept`].
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct TrustWatermarks {
    pub host_pub: Option<Vec<u8>>,
    pub manifest_gen_rev: Option<GenRev>,
    pub last_world_seal: Option<String>,
}

/// Why a sample was discarded (never Leave).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DiscardReason {
    Aead,
    Limits,
    Decode,
    Identity,
    Seal,
    Attest,
    GenRev,
}

/// Result of processing one Manifest sample. Leave is never returned.
#[derive(Debug)]
pub enum TrustOutcome {
    Accept {
        generation_changed: bool,
        manifest: SceneManifest,
    },
    Discard {
        reason: DiscardReason,
        detail: String,
    },
}

/// AEAD-open a Manifest envelope with the room key (fail closed before protobuf).
pub fn open_manifest_aead(
    room_id: &str,
    room_key: &[u8],
    envelope: &[u8],
) -> Result<Vec<u8>, String> {
    if let Err(err) = wire_aead::check_aead_envelope_bytes(AeadKind::Manifest, envelope.len()) {
        return Err(err.to_string());
    }
    wire_aead::open(room_key, room_id, 0, AeadKind::Manifest, envelope).map_err(|e| e.to_string())
}

/// Full gate: AEAD → plaintext pipeline. Discard on open failure.
pub fn process_manifest_bytes(
    room_id: &str,
    room_key: &[u8],
    envelope: &[u8],
    expected_room: &str,
    watermarks: &mut TrustWatermarks,
    seal: &ManifestSealVerifier,
    attest: &HostAttestationVerifier,
) -> TrustOutcome {
    let plaintext = match open_manifest_aead(room_id, room_key, envelope) {
        Ok(pt) => pt,
        Err(detail) => {
            return TrustOutcome::Discard {
                reason: DiscardReason::Aead,
                detail,
            };
        }
    };
    process_manifest_plaintext(&plaintext, expected_room, watermarks, seal, attest)
}

/// Post-AEAD Manifest gate (identity → seal → attest → gen-rev → watermark).
pub fn process_manifest_plaintext(
    plaintext: &[u8],
    expected_room: &str,
    watermarks: &mut TrustWatermarks,
    seal: &ManifestSealVerifier,
    attest: &HostAttestationVerifier,
) -> TrustOutcome {
    if let Err(err) = check_payload_bytes(WireClass::Manifest, plaintext.len()) {
        return TrustOutcome::Discard {
            reason: DiscardReason::Limits,
            detail: err.to_string(),
        };
    }
    let manifest = match SceneManifest::decode(plaintext) {
        Ok(m) => m,
        Err(err) => {
            return TrustOutcome::Discard {
                reason: DiscardReason::Decode,
                detail: err.to_string(),
            };
        }
    };
    if let Err(err) = check_manifest(&manifest) {
        return TrustOutcome::Discard {
            reason: DiscardReason::Limits,
            detail: err.to_string(),
        };
    }
    if let Err(reason) = gate_manifest_identity(
        &manifest.protocol_pin,
        &manifest.room_id,
        manifest.host_pub.len(),
        expected_room,
    ) {
        return TrustOutcome::Discard {
            reason: DiscardReason::Identity,
            detail: reason.to_string(),
        };
    }
    let body_hex = manifest_seal_blake3_hex(&manifest);
    if let Err(err) = seal.verify(
        &manifest.world_seal,
        manifest.revision,
        manifest.host_generation,
        &body_hex,
    ) {
        return TrustOutcome::Discard {
            reason: DiscardReason::Seal,
            detail: format!("{err:#}"),
        };
    }
    let (attested_pub, attested_gen) = match attest.verify_and_host_pub(
        &manifest.host_attestation,
        &manifest.host_pub,
        manifest.host_generation,
    ) {
        Ok(pair) => pair,
        Err(err) => {
            return TrustOutcome::Discard {
                reason: DiscardReason::Attest,
                detail: format!("{err:#}"),
            };
        }
    };
    let incoming = GenRev {
        host_generation: attested_gen,
        revision: manifest.revision,
    };
    let known = watermarks.manifest_gen_rev;
    let decision = decide_gen_rev(known, incoming);
    let generation_changed = match decision {
        GenRevDecision::RejectRegression | GenRevDecision::RejectInvalidGeneration => {
            return TrustOutcome::Discard {
                reason: DiscardReason::GenRev,
                detail: format!(
                    "{decision:?} (incoming gen={} rev={} known={known:?})",
                    incoming.host_generation, incoming.revision
                ),
            };
        }
        GenRevDecision::AcceptSameOrForward => match watermarks.host_pub.as_ref() {
            Some(prev) if prev == &attested_pub => false,
            Some(_) => {
                return TrustOutcome::Discard {
                    reason: DiscardReason::GenRev,
                    detail: format!(
                        "same-generation host_pub mismatch gen={}",
                        incoming.host_generation
                    ),
                };
            }
            None => {
                return TrustOutcome::Discard {
                    reason: DiscardReason::GenRev,
                    detail: format!(
                        "same-generation host_pub missing gen={}",
                        incoming.host_generation
                    ),
                };
            }
        },
        GenRevDecision::AcceptFirst | GenRevDecision::AcceptNewGeneration => {
            let changed = matches!(decision, GenRevDecision::AcceptNewGeneration);
            watermarks.host_pub = Some(attested_pub);
            changed
        }
    };
    watermarks.manifest_gen_rev = Some(incoming);
    watermarks.last_world_seal = Some(manifest.world_seal.clone());
    TrustOutcome::Accept {
        generation_changed,
        manifest,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use alchemy_protocol::generation_gate::GenRev;
    use alchemy_protocol::PROTOCOL_PIN;
    use base64::Engine;
    use jsonwebtoken::{encode, Algorithm, DecodingKey, EncodingKey, Header};
    use pkcs1::EncodeRsaPrivateKey;
    use pkcs8::EncodePublicKey;
    use rsa::RsaPrivateKey;
    use serde::Serialize;
    use std::sync::Arc;
    use std::time::{SystemTime, UNIX_EPOCH};

    fn empty_verifiers(room: &str) -> (ManifestSealVerifier, HostAttestationVerifier) {
        let jwks = Arc::new(crate::WorldJwks::new("http://127.0.0.1:9"));
        (
            ManifestSealVerifier::new(Arc::clone(&jwks), room),
            HostAttestationVerifier::new(jwks, room),
        )
    }

    fn joined_watermarks(host_pub: [u8; 32], gen: u64, rev: u64, seal: &str) -> TrustWatermarks {
        TrustWatermarks {
            host_pub: Some(host_pub.to_vec()),
            manifest_gen_rev: Some(GenRev {
                host_generation: gen,
                revision: rev,
            }),
            last_world_seal: Some(seal.into()),
        }
    }

    #[test]
    fn fake_pin_manifest_discards_identity_without_mutating_watermarks() {
        let (seal, attest) = empty_verifiers("local");
        let mut wm = TrustWatermarks::default();
        let before = wm.clone();
        let manifest = SceneManifest {
            room_id: "local".into(),
            revision: 0,
            entries: vec![],
            locomotion: None,
            protocol_pin: "bogus-pin-x0070".into(),
            host_pub: vec![0u8; 32],
            host_attestation: String::new(),
            world_seal: String::new(),
            host_generation: 0,
        };
        let bytes = manifest.encode_to_vec();
        match process_manifest_plaintext(&bytes, "local", &mut wm, &seal, &attest) {
            TrustOutcome::Discard {
                reason: DiscardReason::Identity,
                ..
            } => {}
            other => panic!("expected Identity discard, got {other:?}"),
        }
        assert_eq!(wm, before);
    }

    #[test]
    fn empty_seal_discards_without_mutating_watermarks() {
        let (seal, attest) = empty_verifiers("local");
        let mut wm = TrustWatermarks::default();
        let before = wm.clone();
        let manifest = SceneManifest {
            room_id: "local".into(),
            revision: 1,
            entries: vec![],
            locomotion: None,
            protocol_pin: PROTOCOL_PIN.into(),
            host_pub: vec![0u8; 32],
            host_attestation: String::new(),
            world_seal: String::new(),
            host_generation: 1,
        };
        let bytes = manifest.encode_to_vec();
        match process_manifest_plaintext(&bytes, "local", &mut wm, &seal, &attest) {
            TrustOutcome::Discard {
                reason: DiscardReason::Seal,
                ..
            } => {}
            other => panic!("expected Seal discard, got {other:?}"),
        }
        assert_eq!(wm, before);
    }

    /// x0070: after a joined Accept watermark, forged seal must Discard without mutation.
    #[test]
    fn joined_fake_seal_discards_without_mutating_watermarks() {
        let (seal, attest) = empty_verifiers("local");
        let host_pub = [7u8; 32];
        let mut wm = joined_watermarks(host_pub, 2, 5, "prior-accepted-seal");
        let before = wm.clone();
        let manifest = SceneManifest {
            room_id: "local".into(),
            revision: 6,
            entries: vec![],
            locomotion: None,
            protocol_pin: PROTOCOL_PIN.into(),
            host_pub: host_pub.to_vec(),
            host_attestation: String::new(),
            world_seal: "eyJhbGciOiJSUzI1NiIsInR5cCI6IkpXVCJ9.e30.e30".into(),
            host_generation: 2,
        };
        let bytes = manifest.encode_to_vec();
        match process_manifest_plaintext(&bytes, "local", &mut wm, &seal, &attest) {
            TrustOutcome::Discard {
                reason: DiscardReason::Seal,
                ..
            } => {}
            other => panic!("expected Seal discard, got {other:?}"),
        }
        assert_eq!(wm, before);
    }

    #[derive(Serialize)]
    struct SealClaims {
        typ: &'static str,
        sub: &'static str,
        room_id: &'static str,
        revision: u64,
        host_generation: u64,
        manifest_blake3: String,
        exp: i64,
        iss: &'static str,
        aud: &'static str,
    }

    #[derive(Serialize)]
    struct AttestClaims {
        typ: &'static str,
        sub: &'static str,
        room_id: &'static str,
        host_pub: String,
        host_generation: u64,
        exp: i64,
        iss: &'static str,
        aud: &'static str,
    }

    fn test_rsa_jwks() -> (Arc<crate::WorldJwks>, EncodingKey) {
        let mut rng = rand::thread_rng();
        let private = RsaPrivateKey::new(&mut rng, 2048).expect("rsa");
        let priv_pem = private
            .to_pkcs1_pem(pkcs1::LineEnding::LF)
            .expect("priv pem");
        let pub_pem = private
            .to_public_key()
            .to_public_key_pem(pkcs8::LineEnding::LF)
            .expect("pub pem");
        let encoding = EncodingKey::from_rsa_pem(priv_pem.as_bytes()).expect("encoding");
        let decoding = DecodingKey::from_rsa_pem(pub_pem.as_bytes()).expect("decoding");
        let jwks = Arc::new(crate::WorldJwks::new("http://127.0.0.1:9"));
        jwks.insert_decoding_key_for_test("t", decoding);
        (jwks, encoding)
    }

    /// Valid seal+attest but revision regression → GenRev Discard; joined watermarks frozen.
    #[test]
    fn joined_gen_rev_regression_discards_without_mutating_watermarks() {
        let (jwks, encoding) = test_rsa_jwks();
        let seal = ManifestSealVerifier::new(Arc::clone(&jwks), "local");
        let attest = HostAttestationVerifier::new(jwks, "local");
        let host_pub = [9u8; 32];
        let mut wm = joined_watermarks(host_pub, 1, 5, "prior-accepted-seal");
        let before = wm.clone();

        let host_b64 = base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(host_pub);
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_secs() as i64;
        let mut header = Header::new(Algorithm::RS256);
        header.kid = Some("t".into());
        let attest_jwt = encode(
            &header,
            &AttestClaims {
                typ: "host_attestation",
                sub: "s",
                room_id: "local",
                host_pub: host_b64,
                host_generation: 1,
                exp: now + 600,
                iss: "alchemy-world",
                aud: "alchemy-host-attestation",
            },
            &encoding,
        )
        .expect("attest jwt");
        // Attestation is part of seal-hash input; set it before hashing, seal after.
        let mut manifest = SceneManifest {
            room_id: "local".into(),
            revision: 3, // regression vs watermark rev=5
            entries: vec![],
            locomotion: None,
            protocol_pin: PROTOCOL_PIN.into(),
            host_pub: host_pub.to_vec(),
            host_attestation: attest_jwt,
            world_seal: String::new(),
            host_generation: 1,
        };
        let body_hex = manifest_seal_blake3_hex(&manifest);
        manifest.world_seal = encode(
            &header,
            &SealClaims {
                typ: "manifest_seal",
                sub: "s",
                room_id: "local",
                revision: 3,
                host_generation: 1,
                manifest_blake3: body_hex,
                exp: now + 600,
                iss: "alchemy-world",
                aud: "alchemy-manifest-seal",
            },
            &encoding,
        )
        .expect("seal jwt");

        let bytes = manifest.encode_to_vec();
        match process_manifest_plaintext(&bytes, "local", &mut wm, &seal, &attest) {
            TrustOutcome::Discard {
                reason: DiscardReason::GenRev,
                ..
            } => {}
            other => panic!("expected GenRev discard, got {other:?}"),
        }
        assert_eq!(wm, before);
    }
}
