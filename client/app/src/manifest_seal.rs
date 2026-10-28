//! Verify world-issued ManifestSeal JWTs (x0026 / x003a).
//!
//! Guarantees: RS256 via shared [`WorldJwks`]; typ/aud/room_id/revision/host_generation/manifest_blake3 match.
//! Non-guarantees: SceneManifest-driven content (P4); offline JWKS beyond soft TTL.

use crate::world_jwks::WorldJwks;
use anyhow::{anyhow, Context, Result};
use jsonwebtoken::{Algorithm, Validation};
use serde::Deserialize;
use std::sync::Arc;

const ISSUER: &str = "alchemy-world";
const AUDIENCE: &str = "alchemy-manifest-seal";
const TYP: &str = "manifest_seal";

#[derive(Debug, Clone, Deserialize)]
pub struct ManifestSealClaims {
    pub typ: String,
    pub sub: String,
    pub room_id: String,
    pub revision: u64,
    pub host_generation: u64,
    pub manifest_blake3: String,
    pub exp: i64,
}

/// ManifestSeal verifier (shares JWKS with HostAttestation).
pub struct ManifestSealVerifier {
    jwks: Arc<WorldJwks>,
    expected_room_id: String,
}

impl ManifestSealVerifier {
    pub fn new(jwks: Arc<WorldJwks>, room_id: &str) -> Self {
        Self {
            jwks,
            expected_room_id: room_id.to_string(),
        }
    }

    /// Verify seal JWT against expected room / revision / generation / body blake3 hex.
    pub fn verify(
        &self,
        seal: &str,
        revision: u64,
        host_generation: u64,
        body_blake3_hex: &str,
    ) -> Result<()> {
        if seal.is_empty() {
            return Err(anyhow!("empty world_seal"));
        }
        let mut validation = Validation::new(Algorithm::RS256);
        validation.set_issuer(&[ISSUER]);
        validation.set_audience(&[AUDIENCE]);
        validation.validate_exp = true;

        let claims: ManifestSealClaims = self
            .jwks
            .decode_claims(seal, &validation)
            .context("verify ManifestSeal")?;
        apply_seal_claims(
            &claims,
            &self.expected_room_id,
            revision,
            host_generation,
            body_blake3_hex,
        )
    }
}

/// Pure claim checks (unit-tested without HTTP). Covers slice success conditions 1–2 core.
pub fn apply_seal_claims(
    claims: &ManifestSealClaims,
    expected_room_id: &str,
    revision: u64,
    host_generation: u64,
    body_blake3_hex: &str,
) -> Result<()> {
    if claims.typ != TYP {
        return Err(anyhow!("seal typ mismatch: {}", claims.typ));
    }
    if claims.room_id != expected_room_id {
        return Err(anyhow!(
            "seal room_id mismatch: {} expected {}",
            claims.room_id,
            expected_room_id
        ));
    }
    if claims.revision != revision {
        return Err(anyhow!(
            "seal revision mismatch: {} expected {}",
            claims.revision,
            revision
        ));
    }
    if claims.host_generation != host_generation {
        return Err(anyhow!(
            "seal host_generation mismatch: {} expected {}",
            claims.host_generation,
            host_generation
        ));
    }
    if claims.host_generation == 0 {
        return Err(anyhow!("seal host_generation must be > 0"));
    }
    let claim_hex = claims.manifest_blake3.to_ascii_lowercase();
    if claim_hex != body_blake3_hex.to_ascii_lowercase() {
        return Err(anyhow!("seal manifest_blake3 mismatch"));
    }
    let _ = claims.sub;
    let _ = claims.exp;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use base64::Engine;
    use jsonwebtoken::DecodingKey;

    fn hex64(byte: u8) -> String {
        format!("{byte:02x}").repeat(32)
    }

    #[test]
    fn empty_seal_rejected() {
        let jwks = Arc::new(WorldJwks::new("http://127.0.0.1:4003"));
        let v = ManifestSealVerifier::new(jwks, "local");
        assert!(v.verify("", 1, 1, &hex64(0xab)).is_err());
    }

    #[test]
    fn fake_jwt_rejected_without_network() {
        let jwks = Arc::new(WorldJwks::new("http://127.0.0.1:9"));
        let n = base64::engine::general_purpose::URL_SAFE_NO_PAD.encode([1u8; 256]);
        let e = base64::engine::general_purpose::URL_SAFE_NO_PAD.encode([1u8, 0, 1]);
        jwks.insert_decoding_key_for_test("t", DecodingKey::from_rsa_components(&n, &e).unwrap());
        let v = ManifestSealVerifier::new(jwks, "local");
        let fake = "eyJhbGciOiJSUzI1NiIsImtpZCI6InQiLCJ0eXAiOiJKV1QifQ.e30.e30";
        assert!(v.verify(fake, 1, 1, &hex64(0xab)).is_err());
    }

    #[test]
    fn claims_reject_blake3_mismatch() {
        let claims = ManifestSealClaims {
            typ: TYP.into(),
            sub: "s".into(),
            room_id: "local".into(),
            revision: 1,
            host_generation: 1,
            manifest_blake3: hex64(0x11),
            exp: 9_999_999_999,
        };
        let err = apply_seal_claims(&claims, "local", 1, 1, &hex64(0x22)).unwrap_err();
        assert!(err.to_string().contains("manifest_blake3 mismatch"));
    }

    #[test]
    fn claims_reject_room_revision_generation_mismatch() {
        let claims = ManifestSealClaims {
            typ: TYP.into(),
            sub: "s".into(),
            room_id: "other".into(),
            revision: 2,
            host_generation: 1,
            manifest_blake3: hex64(0xab),
            exp: 9_999_999_999,
        };
        assert!(apply_seal_claims(&claims, "local", 2, 1, &hex64(0xab))
            .unwrap_err()
            .to_string()
            .contains("room_id"));
        let claims = ManifestSealClaims {
            room_id: "local".into(),
            ..claims
        };
        assert!(apply_seal_claims(&claims, "local", 1, 1, &hex64(0xab))
            .unwrap_err()
            .to_string()
            .contains("revision"));
        assert!(apply_seal_claims(&claims, "local", 2, 9, &hex64(0xab))
            .unwrap_err()
            .to_string()
            .contains("host_generation"));
    }

    #[test]
    fn claims_accept_matching() {
        let hex = hex64(0xcd);
        let claims = ManifestSealClaims {
            typ: TYP.into(),
            sub: "s".into(),
            room_id: "local".into(),
            revision: 3,
            host_generation: 2,
            manifest_blake3: hex.to_ascii_uppercase(),
            exp: 9_999_999_999,
        };
        assert!(apply_seal_claims(&claims, "local", 3, 2, &hex).is_ok());
    }
}
