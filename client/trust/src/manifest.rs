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
    wire_aead::open(room_key, room_id, AeadKind::Manifest, envelope).map_err(|e| e.to_string())
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
    use alchemy_protocol::PROTOCOL_PIN;
    use std::sync::Arc;

    fn empty_verifiers(room: &str) -> (ManifestSealVerifier, HostAttestationVerifier) {
        let jwks = Arc::new(crate::WorldJwks::new("http://127.0.0.1:9"));
        (
            ManifestSealVerifier::new(Arc::clone(&jwks), room),
            HostAttestationVerifier::new(jwks, room),
        )
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
        let _ = PROTOCOL_PIN;
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
}
