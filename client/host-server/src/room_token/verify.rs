//! Room Token claims + RS256 verify against cached JWKS (no HTTP on the verify path).
//!
//! Type + claim checks live here; JWKS fetch / SWR / hard-stale / background kick are in `jwks`
//! (same struct, `pub(super)` fields).
//!
//! Guarantees: RS256 + iss/aud/exp/room_id/role checks; `cnf.spk` shape (x003e); empty/unknown-kid
//! and hard-stale cache fail closed. Cache refresh lives in `jwks`.
//! Non-guarantees: JWKS fetching, credential HTTP.
//! Fuzz: `host-server/fuzz` `room_token_verify` (x004c; feature `fuzzing`).

use super::jwks::{KeyCache, JWKS_HARD_TTL, JWKS_SOFT_TTL};
use anyhow::{anyhow, Context, Result};
#[cfg(any(test, feature = "fuzzing"))]
use jsonwebtoken::DecodingKey;
use jsonwebtoken::{decode, decode_header, Algorithm, Validation};
use serde::Deserialize;
use std::collections::HashMap;
use std::sync::atomic::AtomicBool;
use std::sync::mpsc::SyncSender;
use std::sync::{Arc, Mutex, RwLock};
use std::time::{Duration, Instant};

const ISSUER: &str = "alchemy-world";
const AUDIENCE: &str = "alchemy-room";

/// Confirmation claim (RFC 7800 `cnf`) — Alchemy binds Intent session_pub (x003e).
#[derive(Debug, Clone, Deserialize)]
pub struct RoomCnf {
    /// base64url (no pad) of 32-byte Ed25519 session public key.
    pub spk: String,
}

#[derive(Debug, Clone, Deserialize)]
pub struct RoomClaims {
    /// Auth user id (UUID). Host routes Intent/Grab per `sub` (x0014).
    pub sub: String,
    pub room_id: String,
    pub role: String,
    /// Room Token id — binds Zenoh liveliness Delete to a verified session (x0022).
    pub jti: String,
    /// Present for JWT exp validation / serde; not read by host logic.
    #[allow(dead_code)]
    pub exp: i64,
    /// Intent session key binding (x003e). Required on bind; HTTP credential paths ignore.
    pub cnf: RoomCnf,
}

pub struct RoomTokenVerifier {
    pub(super) jwks_url: String,
    pub(super) expected_room_id: String,
    pub(super) cache: RwLock<KeyCache>,
    pub(super) refresh_tx: Mutex<Option<SyncSender<()>>>,
    pub(super) bg_started: AtomicBool,
    /// Optional x0049 host metrics (JWKS age / hard-stale).
    pub(super) metrics: Mutex<Option<Arc<crate::metrics::HostMetrics>>>,
}

impl RoomTokenVerifier {
    pub fn new(world_url: &str, room_id: &str) -> Self {
        let base = world_url.trim_end_matches('/');
        Self {
            jwks_url: format!("{base}/.well-known/jwks.json"),
            expected_room_id: room_id.to_string(),
            cache: RwLock::new(KeyCache {
                keys: HashMap::new(),
                fetched_at: Instant::now() - Duration::from_secs(3600),
                last_unknown_kid_kick: None,
                last_hard_stale_kick: None,
            }),
            refresh_tx: Mutex::new(None),
            bg_started: AtomicBool::new(false),
            metrics: Mutex::new(None),
        }
    }

    /// Verify using cached keys only (no HTTP). Soft-stale cache still accepts (SWR);
    /// hard-stale cache rejects until background refresh (x0040).
    pub fn verify(&self, token: &str) -> Result<RoomClaims> {
        if token.is_empty() {
            return Err(anyhow!("empty room token"));
        }

        let header = decode_header(token).context("decode JWT header")?;
        let kid = header
            .kid
            .ok_or_else(|| anyhow!("JWT header missing kid"))?;

        {
            let cache = self.cache.read().expect("jwks cache lock");
            if cache.keys.is_empty() {
                return Err(anyhow!("JWKS cache empty; call refresh_jwks before verify"));
            }
            let age = cache.fetched_at.elapsed();
            if age > JWKS_HARD_TTL {
                drop(cache);
                self.with_metrics(|m| m.inc_jwks_hard_stale());
                self.kick_refresh_hard_stale();
                return Err(anyhow!(
                    "JWKS cache hard-stale (age > {}s); refresh required (x0040)",
                    JWKS_HARD_TTL.as_secs()
                ));
            }
            if age > JWKS_SOFT_TTL {
                drop(cache);
                self.kick_refresh();
            }
        }

        if !self.has_kid(&kid) {
            self.kick_refresh_unknown_kid();
            return Err(anyhow!("unknown kid: {kid}"));
        }

        let mut validation = Validation::new(Algorithm::RS256);
        validation.set_issuer(&[ISSUER]);
        validation.set_audience(&[AUDIENCE]);
        validation.validate_exp = true;

        let cache = self.cache.read().expect("jwks cache lock");
        let key = cache
            .keys
            .get(&kid)
            .ok_or_else(|| anyhow!("unknown kid: {kid}"))?;
        let data = decode::<RoomClaims>(token, key, &validation).context("verify Room Token")?;
        if data.claims.room_id != self.expected_room_id {
            return Err(anyhow!(
                "room_id mismatch: token={} expected={}",
                data.claims.room_id,
                self.expected_room_id
            ));
        }
        if data.claims.role != "host" && data.claims.role != "guest" {
            return Err(anyhow!("invalid role: {}", data.claims.role));
        }
        // x003e: reject malformed cnf.spk at verify (HTTP credential paths ignore the bytes).
        crate::session::decode_cnf_spk(&data.claims.cnf.spk)
            .context("Room Token cnf.spk invalid")?;
        Ok(data.claims)
    }

    /// Test / fuzz: inject a DecodingKey so `verify` works without HTTP JWKS.
    #[cfg(any(test, feature = "fuzzing"))]
    pub fn insert_decoding_key_for_test(&self, kid: &str, key: DecodingKey) {
        let mut cache = self.cache.write().expect("jwks cache lock");
        cache.keys.insert(kid.to_string(), key);
        cache.fetched_at = Instant::now();
    }

    /// Test-only: age the JWKS cache (for soft/hard stale tests).
    #[cfg(test)]
    pub fn set_fetched_at_age_for_test(&self, age: Duration) {
        let mut cache = self.cache.write().expect("jwks cache lock");
        cache.fetched_at = Instant::now()
            .checked_sub(age)
            .expect("fetched_at age within Instant range");
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use base64::Engine;

    #[test]
    fn verify_rejects_empty_token() {
        let verifier = RoomTokenVerifier::new("http://127.0.0.1:4003", "local");
        assert!(verifier.verify("").is_err());
    }

    #[test]
    fn verify_does_not_http_when_cache_empty() {
        let verifier = RoomTokenVerifier::new("http://127.0.0.1:9", "local");
        let err = verifier
            .verify("eyJhbGciOiJSUzI1NiIsImtpZCI6InQiLCJ0eXAiOiJKV1QifQ.e30.e30")
            .unwrap_err()
            .to_string();
        assert!(
            err.contains("JWKS cache empty"),
            "verify must not reach network; got: {err}"
        );
    }

    #[derive(Clone)]
    struct TestJwtOpts {
        expected_room: String,
        token_room: String,
        kid: String,
        iss: String,
        aud: String,
        role: String,
        /// Seconds relative to now for `exp` (negative = already expired).
        exp_offset_secs: i64,
        alg: Algorithm,
    }

    impl Default for TestJwtOpts {
        fn default() -> Self {
            Self {
                expected_room: "local".into(),
                token_room: "local".into(),
                kid: "hard-kid".into(),
                iss: "alchemy-world".into(),
                aud: "alchemy-room".into(),
                role: "guest".into(),
                exp_offset_secs: 600,
                alg: Algorithm::RS256,
            }
        }
    }

    fn test_rsa_jwt_with(opts: TestJwtOpts) -> (RoomTokenVerifier, String) {
        use jsonwebtoken::{encode, EncodingKey, Header};
        use pkcs1::EncodeRsaPrivateKey;
        use pkcs8::EncodePublicKey;
        use rsa::RsaPrivateKey;
        use serde::Serialize;
        use std::time::{SystemTime, UNIX_EPOCH};

        let mut rng = rand::thread_rng();
        let private = RsaPrivateKey::new(&mut rng, 2048).expect("rsa");
        let priv_pem = private
            .to_pkcs1_pem(pkcs1::LineEnding::LF)
            .expect("priv pem");
        let pub_pem = private
            .to_public_key()
            .to_public_key_pem(pkcs8::LineEnding::LF)
            .expect("pub pem");
        let encoding = EncodingKey::from_rsa_pem(priv_pem.as_bytes()).expect("enc");
        let decoding = DecodingKey::from_rsa_pem(pub_pem.as_bytes()).expect("dec");
        let verifier = RoomTokenVerifier::new("http://127.0.0.1:9", &opts.expected_room);
        verifier.insert_decoding_key_for_test("hard-kid", decoding);

        #[derive(Serialize)]
        struct Cnf {
            spk: String,
        }
        #[derive(Serialize)]
        struct Claims {
            sub: String,
            jti: String,
            room_id: String,
            role: String,
            exp: usize,
            iat: usize,
            iss: String,
            aud: String,
            cnf: Cnf,
        }
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_secs() as i64;
        let exp = (now + opts.exp_offset_secs).max(0) as usize;
        let mut header = Header::new(opts.alg);
        header.kid = Some(opts.kid.clone());
        let spk = base64::engine::general_purpose::URL_SAFE_NO_PAD.encode([7u8; 32]);
        let token = encode(
            &header,
            &Claims {
                sub: "u-hard".into(),
                jti: "j-hard".into(),
                room_id: opts.token_room,
                role: opts.role,
                exp,
                iat: now as usize,
                iss: opts.iss,
                aud: opts.aud,
                cnf: Cnf { spk },
            },
            &encoding,
        )
        .expect("jwt");
        (verifier, token)
    }

    fn test_rsa_jwt(room_id: &str) -> (RoomTokenVerifier, String) {
        test_rsa_jwt_with(TestJwtOpts {
            expected_room: room_id.into(),
            token_room: room_id.into(),
            ..TestJwtOpts::default()
        })
    }

    #[test]
    fn jwks_soft_accepts_hard_rejects_until_fresh() {
        // One RSA fixture covers soft SWR + hard fail-closed (avoid dual 2048 gen).
        let (verifier, token) = test_rsa_jwt("local");
        verifier.set_fetched_at_age_for_test(Duration::from_secs(61));
        assert!(
            verifier.verify(&token).is_ok(),
            "soft-stale must still accept (SWR)"
        );
        verifier.set_fetched_at_age_for_test(Duration::from_secs(301));
        let err = verifier.verify(&token).expect_err("hard-stale");
        assert!(err.to_string().contains("hard-stale"), "got: {err:#}");
        verifier.set_fetched_at_age_for_test(Duration::from_secs(0));
        assert!(verifier.verify(&token).is_ok());
    }

    #[test]
    fn hard_stale_increments_host_metrics() {
        let (verifier, token) = test_rsa_jwt("local");
        let metrics = Arc::new(crate::metrics::HostMetrics::default());
        verifier.set_metrics(Arc::clone(&metrics));
        verifier.set_fetched_at_age_for_test(Duration::from_secs(301));
        let _ = verifier.verify(&token).expect_err("hard-stale");
        assert_eq!(metrics.jwks_hard_stale_total(), 1);
    }

    // --- x0045 / P3-8: Room Token claim negative tests (shared RSA fixture) ---

    struct SharedRsaFixture {
        verifier: RoomTokenVerifier,
        encoding: jsonwebtoken::EncodingKey,
    }

    fn shared_rsa_fixture() -> SharedRsaFixture {
        use jsonwebtoken::{DecodingKey, EncodingKey};
        use pkcs1::EncodeRsaPrivateKey;
        use pkcs8::EncodePublicKey;
        use rsa::RsaPrivateKey;

        let mut rng = rand::thread_rng();
        let private = RsaPrivateKey::new(&mut rng, 2048).expect("rsa");
        let priv_pem = private
            .to_pkcs1_pem(pkcs1::LineEnding::LF)
            .expect("priv pem");
        let pub_pem = private
            .to_public_key()
            .to_public_key_pem(pkcs8::LineEnding::LF)
            .expect("pub pem");
        let encoding = EncodingKey::from_rsa_pem(priv_pem.as_bytes()).expect("enc");
        let decoding = DecodingKey::from_rsa_pem(pub_pem.as_bytes()).expect("dec");
        let verifier = RoomTokenVerifier::new("http://127.0.0.1:9", "local");
        verifier.insert_decoding_key_for_test("hard-kid", decoding);
        SharedRsaFixture { verifier, encoding }
    }

    fn encode_room_token(encoding: &jsonwebtoken::EncodingKey, opts: &TestJwtOpts) -> String {
        use jsonwebtoken::{encode, Header};
        use serde::Serialize;
        use std::time::{SystemTime, UNIX_EPOCH};

        #[derive(Serialize)]
        struct Cnf {
            spk: String,
        }
        #[derive(Serialize)]
        struct Claims {
            sub: String,
            jti: String,
            room_id: String,
            role: String,
            exp: usize,
            iat: usize,
            iss: String,
            aud: String,
            cnf: Cnf,
        }
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_secs() as i64;
        let exp = (now + opts.exp_offset_secs).max(0) as usize;
        let mut header = Header::new(opts.alg);
        header.kid = Some(opts.kid.clone());
        let spk = base64::engine::general_purpose::URL_SAFE_NO_PAD.encode([7u8; 32]);
        encode(
            &header,
            &Claims {
                sub: "u-hard".into(),
                jti: "j-hard".into(),
                room_id: opts.token_room.clone(),
                role: opts.role.clone(),
                exp,
                iat: now as usize,
                iss: opts.iss.clone(),
                aud: opts.aud.clone(),
                cnf: Cnf { spk },
            },
            encoding,
        )
        .expect("jwt")
    }

    fn assert_err_chain_contains(err: &anyhow::Error, needle: &str) {
        let chain = format!("{err:#}");
        assert!(
            chain.contains(needle),
            "expected chain to contain `{needle}`, got: {chain}"
        );
    }

    #[test]
    fn verify_rejects_claim_negatives_x0045() {
        // One RSA 2048 for the whole matrix (CI time).
        let SharedRsaFixture { verifier, encoding } = shared_rsa_fixture();

        // alg=none (unsigned) with cached kid
        {
            let header = base64::engine::general_purpose::URL_SAFE_NO_PAD
                .encode(br#"{"alg":"none","kid":"hard-kid"}"#);
            let payload = base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(
                br#"{"sub":"u","jti":"j","room_id":"local","role":"guest","exp":9999999999,"iat":1,"iss":"alchemy-world","aud":"alchemy-room","cnf":{"spk":"AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA"}}"#,
            );
            let token = format!("{header}.{payload}.");
            let err = verifier.verify(&token).expect_err("alg none");
            let chain = format!("{err:#}");
            // jsonwebtoken rejects `alg:none` at header deserialize (not at RS256 verify).
            assert!(
                chain.contains("unknown variant `none`")
                    || chain.contains("decode JWT header")
                    || chain.contains("InvalidAlgorithm"),
                "alg none: {chain}"
            );
        }

        // alg=HS256 against RS256 validation
        {
            use jsonwebtoken::{encode, EncodingKey, Header};
            use serde::Serialize;
            use std::time::{SystemTime, UNIX_EPOCH};
            #[derive(Serialize)]
            struct Cnf {
                spk: String,
            }
            #[derive(Serialize)]
            struct Claims {
                sub: String,
                jti: String,
                room_id: String,
                role: String,
                exp: usize,
                iat: usize,
                iss: String,
                aud: String,
                cnf: Cnf,
            }
            let now = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_secs() as usize;
            let mut header = Header::new(Algorithm::HS256);
            header.kid = Some("hard-kid".into());
            let hs = EncodingKey::from_secret(b"not-an-rsa-key");
            let spk = base64::engine::general_purpose::URL_SAFE_NO_PAD.encode([7u8; 32]);
            let token = encode(
                &header,
                &Claims {
                    sub: "u".into(),
                    jti: "j".into(),
                    room_id: "local".into(),
                    role: "guest".into(),
                    exp: now + 600,
                    iat: now,
                    iss: "alchemy-world".into(),
                    aud: "alchemy-room".into(),
                    cnf: Cnf { spk },
                },
                &hs,
            )
            .expect("hs256 jwt");
            let err = verifier.verify(&token).expect_err("hs256");
            let chain = format!("{err:#}");
            assert!(
                chain.contains("InvalidAlgorithm") || chain.contains("Algorithm"),
                "hs256: {chain}"
            );
        }

        let err = verifier
            .verify(&encode_room_token(
                &encoding,
                &TestJwtOpts {
                    iss: "evil-issuer".into(),
                    ..TestJwtOpts::default()
                },
            ))
            .expect_err("wrong iss");
        assert_err_chain_contains(&err, "InvalidIssuer");

        let err = verifier
            .verify(&encode_room_token(
                &encoding,
                &TestJwtOpts {
                    aud: "alchemy-user".into(),
                    ..TestJwtOpts::default()
                },
            ))
            .expect_err("wrong aud");
        assert_err_chain_contains(&err, "InvalidAudience");

        let err = verifier
            .verify(&encode_room_token(
                &encoding,
                &TestJwtOpts {
                    exp_offset_secs: -120,
                    ..TestJwtOpts::default()
                },
            ))
            .expect_err("expired");
        assert_err_chain_contains(&err, "ExpiredSignature");

        let err = verifier
            .verify(&encode_room_token(
                &encoding,
                &TestJwtOpts {
                    token_room: "other".into(),
                    ..TestJwtOpts::default()
                },
            ))
            .expect_err("room mismatch");
        assert_err_chain_contains(&err, "room_id mismatch");

        let err = verifier
            .verify(&encode_room_token(
                &encoding,
                &TestJwtOpts {
                    kid: "no-such-kid".into(),
                    ..TestJwtOpts::default()
                },
            ))
            .expect_err("unknown kid");
        assert_err_chain_contains(&err, "unknown kid");

        let err = verifier
            .verify(&encode_room_token(
                &encoding,
                &TestJwtOpts {
                    role: "admin".into(),
                    ..TestJwtOpts::default()
                },
            ))
            .expect_err("bad role");
        assert_err_chain_contains(&err, "invalid role");
    }
}
