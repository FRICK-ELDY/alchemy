//! Intent session binding (x0023 P2-2) + role→capability (x002a) + cnf (x003e).
//!
//! Guarantees: Room Token alone cannot forge steady Intent; seq must advance;
//! bind requires JWT + matching Ed25519 sig + `cnf.spk` == `session_pub`;
//! bind stores Intent caps **and JWT `role`** from `role` (frozen until re-bind —
//! mid-session role change is non-goal). Stolen JWT cannot bind with a different
//! session key. Inventory control Dual (x0081 / x0098): **`(role=host ∧ sub ==`
//! current host Token sub)** **or** **`sub == room.owner_sub`** (guest bind OK for
//! owner; `owner_sub` missing → fail-closed host-role path only).
//! x0040: [`SessionTable::evaluate_*`] does not mutate; callers rate-limit then
//! [`commit_*`] so forged sigs cannot drain buckets and rate drops do not advance seq.
//! Non-guarantees: Zenoh ACL; client private-key theft; same-key last-wins rebind;
//! Invite / arbitrary-guest ACL; HTTP caps (world-server enforces `role == "host"`;
//! table in `sim_capability`).

use std::collections::HashMap;

use alchemy_protocol::sim_capability::intent_caps_for_role;
use alchemy_protocol::wire_crypto::{self, PUBLIC_KEY_LENGTH};
use alchemy_protocol::InputEvent;
use anyhow::{anyhow, bail, Context, Result};
use base64::Engine;

use crate::room_token::{RoomClaims, RoomTokenVerifier};

pub(crate) fn decode_cnf_spk(spk: &str) -> Result<Vec<u8>> {
    let bytes = base64::engine::general_purpose::URL_SAFE_NO_PAD
        .decode(spk)
        .context("cnf.spk base64url decode")?;
    if bytes.len() != PUBLIC_KEY_LENGTH {
        bail!(
            "cnf.spk length {}, expected {PUBLIC_KEY_LENGTH}",
            bytes.len()
        );
    }
    Ok(bytes)
}

#[derive(Debug, Clone)]
pub struct SessionBinding {
    pub sub: String,
    pub jti: String,
    pub last_seq: u64,
    /// Inventory control-plane seq (x0081); independent of Intent `intent_seq`.
    pub last_control_seq: u64,
    /// Intent-path capability bits from Room Token `role` at bind (x002a).
    pub caps: u32,
    /// Room Token `role` at bind (`host` / `guest`); host-role path of control Dual (x0081 / x0098).
    pub role: String,
}

/// Bind path after Ed25519 + JWT; not yet written to the table.
#[derive(Debug, Clone)]
pub struct PendingBind {
    pub sub: String,
    pub jti: String,
    pub caps: u32,
    pub role: String,
}

/// Steady path after Ed25519 + seq check; `last_seq` not yet advanced.
#[derive(Debug, Clone)]
pub struct PendingSteady {
    pub sub: String,
    pub jti: String,
    pub caps: u32,
}

#[derive(Debug, Default)]
pub struct SessionTable {
    /// `session_pub` bytes → binding.
    by_pub: HashMap<Vec<u8>, SessionBinding>,
}

impl SessionTable {
    #[cfg(test)]
    fn get(&self, session_pub: &[u8]) -> Option<&SessionBinding> {
        self.by_pub.get(session_pub)
    }

    /// True if `session_pub` already has a Session binding (successful prior bind).
    pub fn is_bound(&self, session_pub: &[u8]) -> bool {
        self.by_pub.contains_key(session_pub)
    }

    /// Remove all bindings for `sub`. Returns removed `session_pub` keys (for rate cleanup).
    pub fn remove_by_sub(&mut self, sub: &str) -> Vec<Vec<u8>> {
        let mut removed = Vec::new();
        self.by_pub.retain(|k, b| {
            if b.sub == sub {
                removed.push(k.clone());
                false
            } else {
                true
            }
        });
        removed
    }

    fn require_session_pub_len(event: &InputEvent) -> Result<()> {
        if event.session_pub.len() != PUBLIC_KEY_LENGTH {
            bail!(
                "session_pub length {}, expected {PUBLIC_KEY_LENGTH}",
                event.session_pub.len()
            );
        }
        Ok(())
    }

    /// Ed25519 over Intent body (shared by bind / steady).
    pub fn verify_intent_sig(event: &InputEvent) -> Result<()> {
        Self::require_session_pub_len(event)?;
        wire_crypto::verify_input(event).map_err(|e| anyhow::anyhow!("intent sig: {e}"))
    }

    /// JWT + bind policy checks; **no table mutation**.
    pub fn evaluate_bind(
        &self,
        event: &InputEvent,
        verifier: &RoomTokenVerifier,
    ) -> Result<PendingBind> {
        Self::require_session_pub_len(event)?;
        if event.room_token.is_empty() {
            bail!("evaluate_bind requires room_token");
        }
        let claims: RoomClaims = verifier.verify(&event.room_token)?;
        if claims.jti.is_empty() || claims.jti.contains('/') {
            bail!("Room Token jti missing or contains '/'");
        }
        if claims.sub.contains('/') {
            bail!("Room Token sub contains '/'");
        }
        let cnf_pub = decode_cnf_spk(&claims.cnf.spk)?;
        if cnf_pub.as_slice() != event.session_pub.as_slice() {
            return Err(anyhow!("cnf.spk does not match Intent session_pub (x003e)"));
        }
        let caps = intent_caps_for_role(&claims.role);
        if caps == 0 {
            bail!("role {:?} has no Intent capabilities", claims.role);
        }
        match self.by_pub.get(&event.session_pub) {
            Some(existing) if existing.sub != claims.sub => {
                bail!(
                    "session_pub rebound to different sub (was {}, now {})",
                    existing.sub,
                    claims.sub
                );
            }
            Some(existing) if event.intent_seq <= existing.last_seq => {
                bail!(
                    "intent_seq {} not after last {}",
                    event.intent_seq,
                    existing.last_seq
                );
            }
            _ => {}
        }
        Ok(PendingBind {
            sub: claims.sub,
            jti: claims.jti,
            caps,
            role: claims.role,
        })
    }

    /// Seq / binding checks; **no `last_seq` update**.
    pub fn evaluate_steady(&self, event: &InputEvent) -> Result<PendingSteady> {
        Self::require_session_pub_len(event)?;
        let Some(binding) = self.by_pub.get(&event.session_pub) else {
            bail!("steady Intent with unknown session_pub (bind with room_token first)");
        };
        if event.intent_seq <= binding.last_seq {
            bail!(
                "intent_seq {} not after last {}",
                event.intent_seq,
                binding.last_seq
            );
        }
        Ok(PendingSteady {
            sub: binding.sub.clone(),
            jti: binding.jti.clone(),
            caps: binding.caps,
        })
    }

    /// Apply bind. Returns `(sub, jti, caps, removed_other_session_pubs)`.
    /// `removed` excludes the pub being bound so rate GC does not reset its bucket.
    /// Same `session_pub` re-bind keeps `last_control_seq` (host JWT upgrade must not
    /// rewind control-plane anti-replay).
    pub fn commit_bind(
        &mut self,
        event: &InputEvent,
        pending: PendingBind,
    ) -> (String, String, u32, Vec<Vec<u8>>) {
        let prior_control_seq = self
            .by_pub
            .get(&event.session_pub)
            .map(|b| b.last_control_seq)
            .unwrap_or(0);
        let mut removed = self.remove_by_sub(&pending.sub);
        removed.retain(|p| p.as_slice() != event.session_pub.as_slice());
        self.by_pub.insert(
            event.session_pub.clone(),
            SessionBinding {
                sub: pending.sub.clone(),
                jti: pending.jti.clone(),
                last_seq: event.intent_seq,
                last_control_seq: prior_control_seq,
                caps: pending.caps,
                role: pending.role.clone(),
            },
        );
        (pending.sub, pending.jti, pending.caps, removed)
    }

    /// Control-plane checks without mutating `last_control_seq` (x0081 / x0098).
    /// Callers rate-limit then [`commit_control`] so forged/unbound traffic cannot burn rate.
    ///
    /// Accepts host-role bind whose `sub` matches `expected_host_sub`, **or** any bound
    /// session whose `sub` matches `owner_sub` (when known). Missing/empty `owner_sub`
    /// fails closed to the host-role path only.
    pub fn evaluate_control(
        &self,
        session_pub: &[u8],
        control_seq: u64,
        expected_host_sub: &str,
        owner_sub: Option<&str>,
    ) -> Result<()> {
        let Some(binding) = self.by_pub.get(session_pub) else {
            bail!("control plane with unknown session_pub (bind Intent first)");
        };
        if control_seq <= binding.last_control_seq {
            bail!(
                "control_seq {} not after last {}",
                control_seq,
                binding.last_control_seq
            );
        }
        let host_ok = binding.role == "host" && binding.sub == expected_host_sub;
        let owner_ok = owner_sub
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .is_some_and(|owner| binding.sub == owner);
        if host_ok || owner_ok {
            return Ok(());
        }
        if binding.role != "host" {
            bail!(
                "control plane requires host-role Intent bind or owner_sub match (got role={}, sub={})",
                binding.role,
                binding.sub
            );
        }
        bail!(
            "control plane sub mismatch (session {}, host {}, owner_sub={:?})",
            binding.sub,
            expected_host_sub,
            owner_sub
        );
    }

    /// Advance `last_control_seq` after rate allow (re-checks binding).
    pub fn commit_control(
        &mut self,
        session_pub: &[u8],
        control_seq: u64,
        expected_host_sub: &str,
        owner_sub: Option<&str>,
    ) -> Result<()> {
        self.evaluate_control(session_pub, control_seq, expected_host_sub, owner_sub)?;
        let binding = self.by_pub.get_mut(session_pub).ok_or_else(|| {
            anyhow!("commit_control: session_pub unbound between evaluate and commit")
        })?;
        binding.last_control_seq = control_seq;
        Ok(())
    }

    /// Test helper: evaluate + commit (no external rate gate).
    #[cfg(test)]
    pub fn authorize_control(
        &mut self,
        session_pub: &[u8],
        control_seq: u64,
        expected_host_sub: &str,
        owner_sub: Option<&str>,
    ) -> Result<()> {
        self.evaluate_control(session_pub, control_seq, expected_host_sub, owner_sub)?;
        self.commit_control(session_pub, control_seq, expected_host_sub, owner_sub)
    }

    /// Advance `last_seq` after rate allow. Err if binding vanished (reclaim race).
    pub fn commit_steady(
        &mut self,
        event: &InputEvent,
        pending: PendingSteady,
    ) -> Result<(String, String, u32)> {
        let Some(binding) = self.by_pub.get_mut(&event.session_pub) else {
            bail!("commit_steady: session_pub unbound between evaluate and commit");
        };
        binding.last_seq = event.intent_seq;
        // jti/caps frozen at bind; keep pending as SSoT for return.
        Ok((pending.sub, pending.jti, pending.caps))
    }

    /// Convenience for tests: evaluate + commit (no external rate gate).
    #[cfg(test)]
    pub fn authorize(
        &mut self,
        event: &InputEvent,
        verifier: &RoomTokenVerifier,
    ) -> Result<(String, String, u32)> {
        Self::verify_intent_sig(event)?;
        if !event.room_token.is_empty() {
            let pending = self.evaluate_bind(event, verifier)?;
            let (sub, jti, caps, _) = self.commit_bind(event, pending);
            Ok((sub, jti, caps))
        } else {
            let pending = self.evaluate_steady(event)?;
            self.commit_steady(event, pending)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use alchemy_protocol::input_buttons;
    use alchemy_protocol::sim_capability::{
        has_any, mask_intent, INTENT_GRAB, INTENT_LEAVE, INTENT_MOVE,
    };
    use alchemy_protocol::wire_crypto::{generate_signing_key, public_key_bytes, sign_input};
    use base64::Engine;
    use jsonwebtoken::{encode, Algorithm, EncodingKey, Header};
    use pkcs1::EncodeRsaPrivateKey;
    use pkcs8::EncodePublicKey;
    use rsa::RsaPrivateKey;
    use serde::Serialize;
    use std::time::{Instant, SystemTime, UNIX_EPOCH};

    use crate::intent_rate::IntentRateLimiter;

    fn guest_caps() -> u32 {
        INTENT_MOVE | INTENT_GRAB | INTENT_LEAVE
    }

    fn unsigned_intent(session_pub: Vec<u8>, seq: u64) -> InputEvent {
        InputEvent {
            room_token: String::new(),
            client_time_ms: 0,
            move_x: 0.0,
            move_y: 0.0,
            move_z: 0.0,
            buttons: 0,
            grab_entity_id: 0,
            aim_x: 0.0,
            aim_y: 0.0,
            aim_z: 0.0,
            leave: false,
            session_pub,
            intent_seq: seq,
            intent_sig: vec![],
        }
    }

    struct TestJwt {
        encoding: EncodingKey,
        verifier: RoomTokenVerifier,
    }

    impl TestJwt {
        fn new(room_id: &str) -> Self {
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
            let decoding =
                jsonwebtoken::DecodingKey::from_rsa_pem(pub_pem.as_bytes()).expect("dec");
            let verifier = RoomTokenVerifier::new("http://127.0.0.1:9", room_id);
            verifier.insert_decoding_key_for_test("test-kid", decoding);
            Self { encoding, verifier }
        }

        fn token(
            &self,
            sub: &str,
            jti: &str,
            role: &str,
            room_id: &str,
            session_pub: &[u8],
        ) -> String {
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
            let mut header = Header::new(Algorithm::RS256);
            header.kid = Some("test-kid".into());
            let spk = base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(session_pub);
            encode(
                &header,
                &Claims {
                    sub: sub.into(),
                    jti: jti.into(),
                    room_id: room_id.into(),
                    role: role.into(),
                    exp: now + 600,
                    iat: now,
                    iss: "alchemy-world".into(),
                    aud: "alchemy-room".into(),
                    cnf: Cnf { spk },
                },
                &self.encoding,
            )
            .expect("jwt")
        }
    }

    #[test]
    fn steady_requires_prior_bind_and_advancing_seq() {
        let mut table = SessionTable::default();
        let key = generate_signing_key();
        let mut event = unsigned_intent(public_key_bytes(&key), 1);
        sign_input(&key, &mut event);
        assert!(table.get(&event.session_pub).is_none());
        table.by_pub.insert(
            event.session_pub.clone(),
            SessionBinding {
                sub: "u1".into(),
                jti: "j1".into(),
                last_seq: 0,
                last_control_seq: 0,
                caps: guest_caps(),
                role: "guest".into(),
            },
        );
        let verifier = RoomTokenVerifier::new("http://127.0.0.1:9", "local");
        let (sub, jti, caps) = table.authorize(&event, &verifier).expect("steady");
        assert_eq!(sub, "u1");
        assert_eq!(jti, "j1");
        assert_eq!(caps, guest_caps());
        assert_eq!(table.get(&event.session_pub).unwrap().last_seq, 1);

        event.intent_seq = 1;
        sign_input(&key, &mut event);
        assert!(table.authorize(&event, &verifier).is_err());
    }

    #[test]
    fn bad_sig_rejected_before_token() {
        let mut table = SessionTable::default();
        let key = generate_signing_key();
        let mut event = unsigned_intent(public_key_bytes(&key), 1);
        event.room_token = "not-jwt".into();
        event.intent_sig = vec![0u8; 64];
        let verifier = RoomTokenVerifier::new("http://127.0.0.1:9", "local");
        assert!(table.authorize(&event, &verifier).is_err());
    }

    #[test]
    fn remove_by_sub_clears_binding() {
        let mut table = SessionTable::default();
        table.by_pub.insert(
            vec![1u8; 32],
            SessionBinding {
                sub: "alice".into(),
                jti: "j".into(),
                last_seq: 2,
                last_control_seq: 0,
                caps: guest_caps(),
                role: "guest".into(),
            },
        );
        table.remove_by_sub("alice");
        assert!(table.by_pub.is_empty());
    }

    #[test]
    fn bind_jwt_fail_forget_uses_is_bound_at_cleanup() {
        // Gate contract (x009c): forget only when SessionTable has no binding.
        // Mirrors main.rs Err path: re-check is_bound → after_bind_jwt_fail.
        let mut table = SessionTable::default();
        let mut rates = IntentRateLimiter::new();
        let t0 = Instant::now();
        let key = generate_signing_key();
        let pub_bytes = public_key_bytes(&key);

        assert!(rates.allow_bind(&pub_bytes, t0));
        assert!(!table.is_bound(&pub_bytes));
        rates.after_bind_jwt_fail(&pub_bytes, table.is_bound(&pub_bytes));
        assert!(
            !rates.test_has_bind_pub(&pub_bytes),
            "unbound JWT fail must forget"
        );

        assert!(rates.allow_bind(&pub_bytes, t0));
        table.by_pub.insert(
            pub_bytes.clone(),
            SessionBinding {
                sub: "alice".into(),
                jti: "j1".into(),
                last_seq: 1,
                last_control_seq: 0,
                caps: guest_caps(),
                role: "guest".into(),
            },
        );
        // Concurrent-commit race: evaluate saw unbound, cleanup sees bound.
        assert!(table.is_bound(&pub_bytes));
        let tokens_before = rates.test_bind_pub_tokens(&pub_bytes, t0);
        rates.after_bind_jwt_fail(&pub_bytes, table.is_bound(&pub_bytes));
        assert!(
            rates.test_has_bind_pub(&pub_bytes),
            "bound (incl. concurrent commit) must keep per-pub"
        );
        assert!((rates.test_bind_pub_tokens(&pub_bytes, t0) - tokens_before).abs() < f64::EPSILON);
    }

    #[test]
    fn bind_via_authorize_replaces_other_pub_for_same_sub() {
        let jwt = TestJwt::new("local");

        let mut table = SessionTable::default();
        let old_key = generate_signing_key();
        let old_pub = public_key_bytes(&old_key);
        table.by_pub.insert(
            old_pub.clone(),
            SessionBinding {
                sub: "alice".into(),
                jti: "jti-old".into(),
                last_seq: 5,
                last_control_seq: 0,
                caps: guest_caps(),
                role: "guest".into(),
            },
        );

        let new_key = generate_signing_key();
        let new_pub = public_key_bytes(&new_key);
        let token = jwt.token("alice", "jti-new", "guest", "local", &new_pub);
        let mut event = unsigned_intent(new_pub.clone(), 1);
        event.room_token = token;
        sign_input(&new_key, &mut event);

        let (sub, jti, caps) = table
            .authorize(&event, &jwt.verifier)
            .expect("bind authorize");
        assert_eq!(sub, "alice");
        assert_eq!(jti, "jti-new");
        assert_eq!(caps, guest_caps());
        assert!(
            table.get(&old_pub).is_none(),
            "old session_pub must be removed on bind"
        );
        let binding = table.get(&event.session_pub).unwrap();
        assert_eq!(binding.jti, "jti-new");
        assert_eq!(binding.caps, guest_caps());
    }

    #[test]
    fn commit_bind_removed_excludes_same_pub_rebind() {
        let mut table = SessionTable::default();
        let pub_bytes = vec![1u8; 32];
        table.by_pub.insert(
            pub_bytes.clone(),
            SessionBinding {
                sub: "alice".into(),
                jti: "j-old".into(),
                last_seq: 3,
                last_control_seq: 0,
                caps: guest_caps(),
                role: "guest".into(),
            },
        );
        let event = unsigned_intent(pub_bytes.clone(), 4);
        let pending = PendingBind {
            sub: "alice".into(),
            jti: "j-new".into(),
            caps: guest_caps(),
            role: "guest".into(),
        };
        let (_sub, _jti, _caps, removed) = table.commit_bind(&event, pending);
        assert!(
            removed.is_empty(),
            "same-key rebind must not list current pub for rate GC, got {removed:?}"
        );
        assert!(table.get(&pub_bytes).is_some());
    }

    #[test]
    fn commit_steady_fails_if_unbound_between_evaluate_and_commit() {
        let mut table = SessionTable::default();
        let pub_bytes = vec![2u8; 32];
        table.by_pub.insert(
            pub_bytes.clone(),
            SessionBinding {
                sub: "u".into(),
                jti: "j".into(),
                last_seq: 0,
                last_control_seq: 0,
                caps: guest_caps(),
                role: "guest".into(),
            },
        );
        let event = unsigned_intent(pub_bytes.clone(), 1);
        let pending = table.evaluate_steady(&event).expect("evaluate");
        table.remove_by_sub("u");
        assert!(table.commit_steady(&event, pending).is_err());
    }

    #[test]
    fn bind_host_role_stores_intent_caps_without_http_bits() {
        let jwt = TestJwt::new("local");
        let mut table = SessionTable::default();
        let key = generate_signing_key();
        let pub_bytes = public_key_bytes(&key);
        let token = jwt.token("owner", "j-host", "host", "local", &pub_bytes);
        let mut event = unsigned_intent(pub_bytes, 1);
        event.room_token = token;
        sign_input(&key, &mut event);

        let (_sub, _jti, caps) = table.authorize(&event, &jwt.verifier).expect("host bind");
        assert_eq!(caps, guest_caps());
        assert!(!has_any(caps, alchemy_protocol::sim_capability::HTTP_SCENE));
    }

    #[test]
    fn bind_rejects_stolen_jwt_with_different_session_pub() {
        let jwt = TestJwt::new("local");
        let victim_key = generate_signing_key();
        let victim_pub = public_key_bytes(&victim_key);
        let stolen = jwt.token("alice", "jti-stolen", "guest", "local", &victim_pub);

        let attacker_key = generate_signing_key();
        let mut event = unsigned_intent(public_key_bytes(&attacker_key), 1);
        event.room_token = stolen;
        sign_input(&attacker_key, &mut event);

        let mut table = SessionTable::default();
        let err = table
            .authorize(&event, &jwt.verifier)
            .expect_err("stolen jwt + attacker key must fail");
        assert!(
            err.to_string().contains("cnf.spk"),
            "expected cnf mismatch, got: {err:#}"
        );
    }

    /// Mirrors main.rs: authorize → mask_intent. Restricted caps must not be a no-op.
    #[test]
    fn authorize_then_mask_respects_restricted_caps() {
        let mut table = SessionTable::default();
        let key = generate_signing_key();
        let pub_bytes = public_key_bytes(&key);
        table.by_pub.insert(
            pub_bytes.clone(),
            SessionBinding {
                sub: "limited".into(),
                jti: "j".into(),
                last_seq: 0,
                last_control_seq: 0,
                caps: INTENT_MOVE,
                role: "guest".into(),
            },
        );

        let mut event = unsigned_intent(pub_bytes, 1);
        event.move_x = 0.8;
        event.buttons = input_buttons::GRAB;
        event.grab_entity_id = 14;
        event.aim_y = 1.0;
        event.leave = true;
        sign_input(&key, &mut event);

        let verifier = RoomTokenVerifier::new("http://127.0.0.1:9", "local");
        let (_sub, _jti, caps) = table.authorize(&event, &verifier).expect("steady");
        assert_eq!(caps, INTENT_MOVE);
        mask_intent(caps, &mut event);
        assert!((event.move_x - 0.8).abs() < f32::EPSILON);
        assert_eq!(event.buttons & input_buttons::GRAB, 0);
        assert_eq!(event.grab_entity_id, 0);
        assert!(!event.leave);
        assert!(!has_any(caps, INTENT_GRAB | INTENT_LEAVE));
    }

    #[test]
    fn forged_sig_cannot_drain_victim_steady_bucket() {
        let mut table = SessionTable::default();
        let victim = generate_signing_key();
        let victim_pub = public_key_bytes(&victim);
        table.by_pub.insert(
            victim_pub.clone(),
            SessionBinding {
                sub: "victim".into(),
                jti: "j".into(),
                last_seq: 0,
                last_control_seq: 0,
                caps: guest_caps(),
                role: "guest".into(),
            },
        );
        let mut rates = IntentRateLimiter::new();
        let t0 = Instant::now();
        let before = rates.steady_tokens("victim", t0);

        let mut forged = unsigned_intent(victim_pub, 1);
        forged.intent_sig = vec![0u8; 64];
        assert!(SessionTable::verify_intent_sig(&forged).is_err());
        // Gate never calls allow_steady on sig failure.
        assert!((rates.steady_tokens("victim", t0) - before).abs() < f64::EPSILON);
        assert_eq!(table.get(&forged.session_pub).unwrap().last_seq, 0);
    }

    #[test]
    fn rate_drop_before_commit_does_not_advance_seq() {
        let mut table = SessionTable::default();
        let key = generate_signing_key();
        let pub_bytes = public_key_bytes(&key);
        table.by_pub.insert(
            pub_bytes.clone(),
            SessionBinding {
                sub: "u".into(),
                jti: "j".into(),
                last_seq: 0,
                last_control_seq: 0,
                caps: guest_caps(),
                role: "guest".into(),
            },
        );
        let mut rates = IntentRateLimiter::new();
        let t0 = Instant::now();
        for _ in 0..(crate::intent_rate::STEADY_CAPACITY as u32) {
            assert!(rates.allow_steady(false, "u", t0));
        }

        let mut event = unsigned_intent(pub_bytes, 1);
        sign_input(&key, &mut event);
        SessionTable::verify_intent_sig(&event).expect("sig");
        let pending = table.evaluate_steady(&event).expect("evaluate");
        assert!(!rates.allow_steady(false, &pending.sub, t0));
        // No commit → seq stays 0.
        assert_eq!(table.get(&event.session_pub).unwrap().last_seq, 0);
    }

    #[test]
    fn authorize_control_requires_host_role_without_owner_sub() {
        let mut table = SessionTable::default();
        let pub_bytes = public_key_bytes(&generate_signing_key());
        table.by_pub.insert(
            pub_bytes.clone(),
            SessionBinding {
                sub: "owner".into(),
                jti: "j".into(),
                last_seq: 1,
                last_control_seq: 0,
                caps: guest_caps(),
                role: "guest".into(),
            },
        );
        let err = table
            .authorize_control(&pub_bytes, 1, "owner", None)
            .expect_err("guest bind must not control without owner_sub");
        assert!(
            err.to_string().contains("host-role") || err.to_string().contains("owner_sub"),
            "expected host-role/owner reject, got: {err:#}"
        );
        assert_eq!(table.get(&pub_bytes).unwrap().last_control_seq, 0);
    }

    #[test]
    fn authorize_control_blank_owner_sub_fails_closed_to_host_role() {
        let mut table = SessionTable::default();
        let pub_bytes = public_key_bytes(&generate_signing_key());
        table.by_pub.insert(
            pub_bytes.clone(),
            SessionBinding {
                sub: "owner".into(),
                jti: "j".into(),
                last_seq: 1,
                last_control_seq: 0,
                caps: guest_caps(),
                role: "guest".into(),
            },
        );
        assert!(
            table
                .authorize_control(&pub_bytes, 1, "compute-host", Some(""))
                .is_err(),
            "empty owner_sub must not open owner path"
        );
        assert!(
            table
                .authorize_control(&pub_bytes, 1, "compute-host", Some("   "))
                .is_err(),
            "whitespace owner_sub must not open owner path"
        );
        assert_eq!(table.get(&pub_bytes).unwrap().last_control_seq, 0);
    }

    #[test]
    fn authorize_control_accepts_guest_when_sub_matches_owner_sub() {
        let mut table = SessionTable::default();
        let pub_bytes = public_key_bytes(&generate_signing_key());
        table.by_pub.insert(
            pub_bytes.clone(),
            SessionBinding {
                sub: "owner".into(),
                jti: "j".into(),
                last_seq: 1,
                last_control_seq: 0,
                caps: guest_caps(),
                role: "guest".into(),
            },
        );
        table
            .authorize_control(&pub_bytes, 1, "compute-host", Some("owner"))
            .expect("owner guest control");
        assert_eq!(table.get(&pub_bytes).unwrap().last_control_seq, 1);

        let other = public_key_bytes(&generate_signing_key());
        table.by_pub.insert(
            other.clone(),
            SessionBinding {
                sub: "stranger".into(),
                jti: "j2".into(),
                last_seq: 1,
                last_control_seq: 0,
                caps: guest_caps(),
                role: "guest".into(),
            },
        );
        assert!(
            table
                .authorize_control(&other, 1, "compute-host", Some("owner"))
                .is_err(),
            "non-owner guest must reject"
        );
    }

    #[test]
    fn authorize_control_rejects_unbound_sub_mismatch_and_stale_seq() {
        let mut table = SessionTable::default();
        let pub_bytes = public_key_bytes(&generate_signing_key());
        assert!(table
            .authorize_control(&pub_bytes, 1, "owner", None)
            .is_err());

        table.by_pub.insert(
            pub_bytes.clone(),
            SessionBinding {
                sub: "owner".into(),
                jti: "j".into(),
                last_seq: 1,
                last_control_seq: 3,
                caps: guest_caps(),
                role: "host".into(),
            },
        );
        let err = table
            .authorize_control(&pub_bytes, 4, "other", None)
            .expect_err("sub mismatch");
        assert!(err.to_string().contains("sub mismatch"));
        assert_eq!(table.get(&pub_bytes).unwrap().last_control_seq, 3);

        let err = table
            .authorize_control(&pub_bytes, 3, "owner", None)
            .expect_err("stale seq");
        assert!(err.to_string().contains("control_seq"));
        assert_eq!(table.get(&pub_bytes).unwrap().last_control_seq, 3);

        table
            .authorize_control(&pub_bytes, 4, "owner", None)
            .expect("host control");
        assert_eq!(table.get(&pub_bytes).unwrap().last_control_seq, 4);
    }

    #[test]
    fn authorize_control_accepts_host_role_when_not_owner() {
        let mut table = SessionTable::default();
        let pub_bytes = public_key_bytes(&generate_signing_key());
        table.by_pub.insert(
            pub_bytes.clone(),
            SessionBinding {
                sub: "compute-host".into(),
                jti: "j".into(),
                last_seq: 1,
                last_control_seq: 0,
                caps: guest_caps(),
                role: "host".into(),
            },
        );
        table
            .authorize_control(&pub_bytes, 1, "compute-host", Some("room-owner"))
            .expect("takeover host control");
        assert_eq!(table.get(&pub_bytes).unwrap().last_control_seq, 1);
    }

    #[test]
    fn bind_host_role_is_visible_on_binding() {
        let jwt = TestJwt::new("local");
        let mut table = SessionTable::default();
        let key = generate_signing_key();
        let pub_bytes = public_key_bytes(&key);
        let token = jwt.token("owner", "j-host", "host", "local", &pub_bytes);
        let mut event = unsigned_intent(pub_bytes.clone(), 1);
        event.room_token = token;
        sign_input(&key, &mut event);
        table.authorize(&event, &jwt.verifier).expect("host bind");
        assert_eq!(table.get(&pub_bytes).unwrap().role, "host");
    }

    #[test]
    fn evaluate_control_does_not_advance_seq() {
        let mut table = SessionTable::default();
        let pub_bytes = public_key_bytes(&generate_signing_key());
        table.by_pub.insert(
            pub_bytes.clone(),
            SessionBinding {
                sub: "owner".into(),
                jti: "j".into(),
                last_seq: 1,
                last_control_seq: 2,
                caps: guest_caps(),
                role: "host".into(),
            },
        );
        table
            .evaluate_control(&pub_bytes, 3, "owner", None)
            .expect("evaluate");
        assert_eq!(table.get(&pub_bytes).unwrap().last_control_seq, 2);
        table
            .commit_control(&pub_bytes, 3, "owner", None)
            .expect("commit");
        assert_eq!(table.get(&pub_bytes).unwrap().last_control_seq, 3);
    }

    #[test]
    fn same_pub_rebind_preserves_last_control_seq() {
        let jwt = TestJwt::new("local");
        let mut table = SessionTable::default();
        let key = generate_signing_key();
        let pub_bytes = public_key_bytes(&key);
        let guest = jwt.token("owner", "j-guest", "guest", "local", &pub_bytes);
        let mut event = unsigned_intent(pub_bytes.clone(), 1);
        event.room_token = guest;
        sign_input(&key, &mut event);
        table.authorize(&event, &jwt.verifier).expect("guest bind");
        table
            .authorize_control(&pub_bytes, 1, "owner", None)
            .expect_err("guest cannot control without owner_sub meta");

        // Simulate prior host control seq then upgrade role via same-pub rebind.
        table.by_pub.get_mut(&pub_bytes).unwrap().last_control_seq = 7;
        table.by_pub.get_mut(&pub_bytes).unwrap().role = "host".into();
        // Downgrade path used in test setup only; real upgrade:
        let host = jwt.token("owner", "j-host", "host", "local", &pub_bytes);
        let mut event = unsigned_intent(pub_bytes.clone(), 2);
        event.room_token = host;
        sign_input(&key, &mut event);
        table.authorize(&event, &jwt.verifier).expect("host rebind");
        assert_eq!(table.get(&pub_bytes).unwrap().role, "host");
        assert_eq!(
            table.get(&pub_bytes).unwrap().last_control_seq,
            7,
            "same-pub rebind must not rewind control_seq"
        );
    }
}
