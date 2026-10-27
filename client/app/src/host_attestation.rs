//! Verify world-issued HostAttestation JWTs (x0025).
//!
//! Guarantees: RS256 via shared [`WorldJwks`]; typ/aud/room_id/host_pub match.
//! Non-guarantees: Manifest body integrity (see ManifestSeal); offline JWKS beyond soft TTL.

use crate::world_jwks::WorldJwks;
use anyhow::{anyhow, Context, Result};
use base64::Engine;
use jsonwebtoken::{Algorithm, Validation};
use serde::Deserialize;
use std::sync::Arc;

const ISSUER: &str = "alchemy-world";
const AUDIENCE: &str = "alchemy-host-attestation";
const TYP: &str = "host_attestation";

#[derive(Debug, Clone, Deserialize)]
pub struct HostAttestationClaims {
    pub typ: String,
    pub sub: String,
    pub room_id: String,
    /// base64url (no pad) of 32-byte Ed25519 pub.
    pub host_pub: String,
    pub exp: i64,
}

/// HostAttestation verifier (shares JWKS with ManifestSeal).
pub struct HostAttestationVerifier {
    jwks: Arc<WorldJwks>,
    expected_room_id: String,
}

impl HostAttestationVerifier {
    pub fn new(jwks: Arc<WorldJwks>, room_id: &str) -> Self {
        Self {
            jwks,
            expected_room_id: room_id.to_string(),
        }
    }

    /// Verify JWT and return raw 32-byte `host_pub` if it matches `manifest_host_pub`.
    pub fn verify_and_host_pub(
        &self,
        attestation: &str,
        manifest_host_pub: &[u8],
    ) -> Result<Vec<u8>> {
        if attestation.is_empty() {
            return Err(anyhow!("empty host_attestation"));
        }
        let mut validation = Validation::new(Algorithm::RS256);
        validation.set_issuer(&[ISSUER]);
        validation.set_audience(&[AUDIENCE]);
        validation.validate_exp = true;

        let claims: HostAttestationClaims = self
            .jwks
            .decode_claims(attestation, &validation)
            .context("verify HostAttestation")?;
        apply_attestation_claims(&claims, &self.expected_room_id, manifest_host_pub)
    }
}

/// Pure claim checks (unit-tested without HTTP).
pub fn apply_attestation_claims(
    claims: &HostAttestationClaims,
    expected_room_id: &str,
    manifest_host_pub: &[u8],
) -> Result<Vec<u8>> {
    if claims.typ != TYP {
        return Err(anyhow!("attestation typ mismatch: {}", claims.typ));
    }
    if claims.room_id != expected_room_id {
        return Err(anyhow!(
            "attestation room_id mismatch: {} expected {}",
            claims.room_id,
            expected_room_id
        ));
    }
    let host_pub = base64::engine::general_purpose::URL_SAFE_NO_PAD
        .decode(claims.host_pub.as_bytes())
        .context("decode attestation host_pub")?;
    if host_pub.len() != 32 {
        return Err(anyhow!(
            "attestation host_pub length {} (want 32)",
            host_pub.len()
        ));
    }
    if host_pub.as_slice() != manifest_host_pub {
        return Err(anyhow!(
            "attestation host_pub does not match Manifest.host_pub"
        ));
    }
    let _ = claims.sub;
    let _ = claims.exp;
    Ok(host_pub)
}

#[cfg(test)]
mod tests {
    use super::*;
    use base64::Engine;
    use jsonwebtoken::DecodingKey;

    #[test]
    fn empty_attestation_rejected() {
        let jwks = Arc::new(WorldJwks::new("http://127.0.0.1:4003"));
        let v = HostAttestationVerifier::new(jwks, "local");
        assert!(v.verify_and_host_pub("", &[0u8; 32]).is_err());
    }

    #[test]
    fn fake_jwt_rejected_without_network() {
        let jwks = Arc::new(WorldJwks::new("http://127.0.0.1:9"));
        // Non-empty cache skips HTTP ensure; signature still fails.
        let n = base64::engine::general_purpose::URL_SAFE_NO_PAD.encode([1u8; 256]);
        let e = base64::engine::general_purpose::URL_SAFE_NO_PAD.encode([1u8, 0, 1]);
        jwks.insert_decoding_key_for_test("t", DecodingKey::from_rsa_components(&n, &e).unwrap());
        let v = HostAttestationVerifier::new(jwks, "local");
        let fake = "eyJhbGciOiJSUzI1NiIsImtpZCI6InQiLCJ0eXAiOiJKV1QifQ.e30.e30";
        assert!(v.verify_and_host_pub(fake, &[0u8; 32]).is_err());
    }

    #[test]
    fn claims_reject_host_pub_mismatch() {
        let host_pub = [7u8; 32];
        let encoded = base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(host_pub);
        let claims = HostAttestationClaims {
            typ: TYP.into(),
            sub: "s".into(),
            room_id: "local".into(),
            host_pub: encoded,
            exp: 9_999_999_999,
        };
        let err = apply_attestation_claims(&claims, "local", &[0u8; 32]).unwrap_err();
        assert!(err.to_string().contains("does not match"));
    }
}
