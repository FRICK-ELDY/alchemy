//! Intent session binding (x0023 P2-2).
//!
//! Guarantees: Room Token alone cannot forge steady Intent; seq must advance;
//! bind requires JWT + matching Ed25519 sig.
//! Non-guarantees: Zenoh ACL; client private-key theft; Token eavesdropping on bind.

use std::collections::HashMap;

use alchemy_protocol::wire_crypto::{self, PUBLIC_KEY_LENGTH};
use alchemy_protocol::InputEvent;
use anyhow::{bail, Result};

use crate::room_token::{RoomClaims, RoomTokenVerifier};

#[derive(Debug, Clone)]
pub struct SessionBinding {
    pub sub: String,
    pub jti: String,
    pub last_seq: u64,
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

    /// Resolve `sub`/`jti` for a verified Intent, updating the session table.
    pub fn authorize(
        &mut self,
        event: &InputEvent,
        verifier: &RoomTokenVerifier,
    ) -> Result<(String, String)> {
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
                },
            );
            return Ok((claims.sub, claims.jti));
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
        Ok((binding.sub.clone(), binding.jti.clone()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use alchemy_protocol::wire_crypto::{generate_signing_key, public_key_bytes, sign_input};

    #[test]
    fn steady_requires_prior_bind_and_advancing_seq() {
        let mut table = SessionTable::default();
        let key = generate_signing_key();
        let mut event = InputEvent {
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
            session_pub: public_key_bytes(&key),
            intent_seq: 1,
            intent_sig: vec![],
        };
        sign_input(&key, &mut event);
        assert!(table.get(&event.session_pub).is_none());
        table.by_pub.insert(
            event.session_pub.clone(),
            SessionBinding {
                sub: "u1".into(),
                jti: "j1".into(),
                last_seq: 0,
            },
        );
        let verifier = RoomTokenVerifier::new("http://127.0.0.1:9", "local");
        let (sub, jti) = table.authorize(&event, &verifier).expect("steady");
        assert_eq!(sub, "u1");
        assert_eq!(jti, "j1");
        assert_eq!(table.get(&event.session_pub).unwrap().last_seq, 1);

        event.intent_seq = 1;
        sign_input(&key, &mut event);
        assert!(table.authorize(&event, &verifier).is_err());
    }

    #[test]
    fn bad_sig_rejected_before_token() {
        let mut table = SessionTable::default();
        let key = generate_signing_key();
        let event = InputEvent {
            room_token: "not-jwt".into(),
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
            session_pub: public_key_bytes(&key),
            intent_seq: 1,
            intent_sig: vec![0u8; 64],
        };
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
            },
        );
        table.remove_by_sub("alice");
        assert!(table.by_pub.is_empty());
    }

    #[test]
    fn bind_via_authorize_replaces_other_pub_for_same_sub() {
        use jsonwebtoken::{encode, Algorithm, EncodingKey, Header};
        use pkcs1::EncodeRsaPrivateKey;
        use pkcs8::EncodePublicKey;
        use rsa::RsaPrivateKey;
        use serde::Serialize;
        use std::time::{SystemTime, UNIX_EPOCH};

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
        let decoding = jsonwebtoken::DecodingKey::from_rsa_pem(pub_pem.as_bytes()).expect("dec");

        let verifier = RoomTokenVerifier::new("http://127.0.0.1:9", "local");
        verifier.insert_decoding_key_for_test("test-kid", decoding);

        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_secs() as usize;
        let mut header = Header::new(Algorithm::RS256);
        header.kid = Some("test-kid".into());
        let token = encode(
            &header,
            &Claims {
                sub: "alice".into(),
                jti: "jti-new".into(),
                room_id: "local".into(),
                role: "guest".into(),
                exp: now + 600,
                iat: now,
                iss: "alchemy-world".into(),
                aud: "alchemy-room".into(),
            },
            &encoding,
        )
        .expect("jwt");

        let mut table = SessionTable::default();
        let old_key = generate_signing_key();
        let old_pub = public_key_bytes(&old_key);
        table.by_pub.insert(
            old_pub.clone(),
            SessionBinding {
                sub: "alice".into(),
                jti: "jti-old".into(),
                last_seq: 5,
            },
        );

        let new_key = generate_signing_key();
        let mut event = InputEvent {
            room_token: token,
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
            session_pub: public_key_bytes(&new_key),
            intent_seq: 1,
            intent_sig: vec![],
        };
        sign_input(&new_key, &mut event);

        let (sub, jti) = table.authorize(&event, &verifier).expect("bind authorize");
        assert_eq!(sub, "alice");
        assert_eq!(jti, "jti-new");
        assert!(
            table.get(&old_pub).is_none(),
            "old session_pub must be removed on bind"
        );
        assert_eq!(table.get(&event.session_pub).unwrap().jti, "jti-new");
    }
}
