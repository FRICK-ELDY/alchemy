//! Intent session binding (x0023 P2-2) + role→capability (x002a).
//!
//! Guarantees: Room Token alone cannot forge steady Intent; seq must advance;
//! bind requires JWT + matching Ed25519 sig; bind stores Intent caps from `role`
//! (frozen until re-bind — mid-session role change is non-goal).
//! Non-guarantees: Zenoh ACL; client private-key theft; Token eavesdropping on bind;
//! HTTP caps (world-server enforces `role == "host"`; table in `sim_capability`).

use std::collections::HashMap;

use alchemy_protocol::sim_capability::intent_caps_for_role;
use alchemy_protocol::wire_crypto::{self, PUBLIC_KEY_LENGTH};
use alchemy_protocol::InputEvent;
use anyhow::{bail, Result};

use crate::room_token::{RoomClaims, RoomTokenVerifier};

#[derive(Debug, Clone)]
pub struct SessionBinding {
    pub sub: String,
    pub jti: String,
    pub last_seq: u64,
    /// Intent-path capability bits from Room Token `role` at bind (x002a).
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

    pub fn remove_by_sub(&mut self, sub: &str) {
        self.by_pub.retain(|_, b| b.sub != sub);
    }

    /// Resolve `sub`/`jti`/`caps` for a verified Intent, updating the session table.
    pub fn authorize(
        &mut self,
        event: &InputEvent,
        verifier: &RoomTokenVerifier,
    ) -> Result<(String, String, u32)> {
        if event.session_pub.len() != PUBLIC_KEY_LENGTH {
            bail!(
                "session_pub length {}, expected {PUBLIC_KEY_LENGTH}",
                event.session_pub.len()
            );
        }
        wire_crypto::verify_input(event).map_err(|e| anyhow::anyhow!("intent sig: {e}"))?;

        if !event.room_token.is_empty() {
            let claims: RoomClaims = verifier.verify(&event.room_token)?;
            if claims.jti.is_empty() || claims.jti.contains('/') {
                bail!("Room Token jti missing or contains '/'");
            }
            if claims.sub.contains('/') {
                bail!("Room Token sub contains '/'");
            }
            let caps = intent_caps_for_role(&claims.role);
            // Defense in depth: verifier already rejects unknown roles; keeps
            // spectator / zero-Intent roles rejectable if verifier widens later.
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
            // One active session per sub: last bind wins; drop other pubs for this sub.
            self.remove_by_sub(&claims.sub);
            self.by_pub.insert(
                event.session_pub.clone(),
                SessionBinding {
                    sub: claims.sub.clone(),
                    jti: claims.jti.clone(),
                    last_seq: event.intent_seq,
                    caps,
                },
            );
            return Ok((claims.sub, claims.jti, caps));
        }

        let Some(binding) = self.by_pub.get_mut(&event.session_pub) else {
            bail!("steady Intent with unknown session_pub (bind with room_token first)");
        };
        if event.intent_seq <= binding.last_seq {
            bail!(
                "intent_seq {} not after last {}",
                event.intent_seq,
                binding.last_seq
            );
        }
        binding.last_seq = event.intent_seq;
        Ok((binding.sub.clone(), binding.jti.clone(), binding.caps))
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
    use jsonwebtoken::{encode, Algorithm, DecodingKey, EncodingKey, Header};
    use pkcs1::EncodeRsaPrivateKey;
    use pkcs8::EncodePublicKey;
    use rsa::RsaPrivateKey;
    use serde::Serialize;
    use std::time::{SystemTime, UNIX_EPOCH};

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
            let decoding = DecodingKey::from_rsa_pem(pub_pem.as_bytes()).expect("dec");
            let verifier = RoomTokenVerifier::new("http://127.0.0.1:9", room_id);
            verifier.insert_decoding_key_for_test("test-kid", decoding);
            Self { encoding, verifier }
        }

        fn token(&self, sub: &str, jti: &str, role: &str, room_id: &str) -> String {
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
            }
            let now = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_secs() as usize;
            let mut header = Header::new(Algorithm::RS256);
            header.kid = Some("test-kid".into());
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
                caps: guest_caps(),
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
                caps: guest_caps(),
            },
        );
        table.remove_by_sub("alice");
        assert!(table.by_pub.is_empty());
    }

    #[test]
    fn bind_via_authorize_replaces_other_pub_for_same_sub() {
        let jwt = TestJwt::new("local");
        let token = jwt.token("alice", "jti-new", "guest", "local");

        let mut table = SessionTable::default();
        let old_key = generate_signing_key();
        let old_pub = public_key_bytes(&old_key);
        table.by_pub.insert(
            old_pub.clone(),
            SessionBinding {
                sub: "alice".into(),
                jti: "jti-old".into(),
                last_seq: 5,
                caps: guest_caps(),
            },
        );

        let new_key = generate_signing_key();
        let mut event = unsigned_intent(public_key_bytes(&new_key), 1);
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
    fn bind_host_role_stores_intent_caps_without_http_bits() {
        let jwt = TestJwt::new("local");
        let token = jwt.token("owner", "j-host", "host", "local");
        let mut table = SessionTable::default();
        let key = generate_signing_key();
        let mut event = unsigned_intent(public_key_bytes(&key), 1);
        event.room_token = token;
        sign_input(&key, &mut event);

        let (_sub, _jti, caps) = table.authorize(&event, &jwt.verifier).expect("host bind");
        assert_eq!(caps, guest_caps());
        assert!(!has_any(caps, alchemy_protocol::sim_capability::HTTP_SCENE));
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
                // MOVE only — regression guard while guest/host share full INTENT_*.
                caps: INTENT_MOVE,
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
}
