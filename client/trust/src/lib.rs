//! Shared client trust gates for Manifest / Presence (x0070).
//!
//! Guarantees: AEAD → identity → seal → attest → gen-rev pipeline returns
//! [`TrustOutcome::Accept`] or [`TrustOutcome::Discard`] only — never Leave.
//! Non-guarantees: GUI/wgpu side effects; cryptographically valid old-seal JWT
//! matrix (unit-tested in [`manifest_seal`]); WAN transport (see trust-probe WAN mode).

mod gate;
mod host_attestation;
mod manifest;
mod manifest_seal;
mod world_jwks;

pub use gate::{gate_manifest_identity, gate_presence_pin};
pub use host_attestation::{
    apply_attestation_claims, HostAttestationClaims, HostAttestationVerifier,
};
pub use manifest::{
    open_manifest_aead, process_manifest_bytes, process_manifest_plaintext, DiscardReason,
    TrustOutcome, TrustWatermarks,
};
pub use manifest_seal::{apply_seal_claims, ManifestSealClaims, ManifestSealVerifier};
pub use world_jwks::{JwksMetrics, WorldJwks};
