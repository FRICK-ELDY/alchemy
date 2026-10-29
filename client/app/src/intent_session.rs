//! Client Intent session key (x0023 / x003e).
//!
//! Guarantees: bind Intent carries Room Token until enough successful puts; steady
//! Intents are Ed25519-signed thereafter. Room Token must be issued for this
//! session_pub (`cnf.spk`, x003e).
//! Non-guarantees: host acceptance ack (Zenoh put ≠ verify); key persistence across restart.

use alchemy_protocol::wire_crypto::{generate_signing_key, sign_input, SigningKey};
use alchemy_protocol::InputEvent;
use base64::Engine;

/// Successful Zenoh puts that must still include `room_token` after join / refresh.
/// Covers put failure retries and brief host reject windows without an ack channel.
const BIND_PUTS_REQUIRED: u32 = 3;

/// Non-auth fields for a signed Intent (keeps `signed_intent` under clippy arg limits).
#[derive(Clone, Copy, Default)]
pub struct IntentBody {
    pub client_time_ms: u64,
    pub move_x: f32,
    pub move_y: f32,
    pub move_z: f32,
    pub buttons: u32,
    pub grab_entity_id: u32,
    pub aim_x: f32,
    pub aim_y: f32,
    pub aim_z: f32,
    pub leave: bool,
}

#[derive(Clone)]
pub struct IntentSession {
    key: SigningKey,
    seq: u64,
    /// Remaining successful puts that must carry `room_token`.
    bind_puts_remaining: u32,
}

impl IntentSession {
    pub fn new() -> Self {
        Self {
            key: generate_signing_key(),
            seq: 0,
            bind_puts_remaining: BIND_PUTS_REQUIRED,
        }
    }

    pub fn mark_bind_pending(&mut self) {
        self.bind_puts_remaining = BIND_PUTS_REQUIRED;
    }

    pub fn bind_pending(&self) -> bool {
        self.bind_puts_remaining > 0
    }

    /// Raw 32-byte Ed25519 public key (Intent `session_pub`).
    pub fn session_pub(&self) -> [u8; 32] {
        self.key.verifying_key().to_bytes()
    }

    /// base64url (no pad) for `POST …/tokens` `session_pub` / JWT `cnf.spk` (x003e).
    pub fn session_pub_b64(&self) -> String {
        base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(self.session_pub())
    }

    /// Call only after Zenoh `put` succeeded for an Intent that included `room_token`.
    pub fn note_bind_put_ok(&mut self) {
        self.bind_puts_remaining = self.bind_puts_remaining.saturating_sub(1);
    }

    /// Build a signed Intent. Does **not** clear bind state (caller does after put OK).
    pub fn signed_intent(&mut self, room_token: Option<&str>, body: IntentBody) -> InputEvent {
        self.seq = self.seq.saturating_add(1);
        let include_token = self.bind_puts_remaining > 0;
        let token = if include_token {
            room_token.unwrap_or("").to_string()
        } else {
            String::new()
        };
        let mut event = InputEvent {
            room_token: token,
            client_time_ms: body.client_time_ms,
            move_x: body.move_x,
            move_y: body.move_y,
            move_z: body.move_z,
            buttons: body.buttons,
            grab_entity_id: body.grab_entity_id,
            aim_x: body.aim_x,
            aim_y: body.aim_y,
            aim_z: body.aim_z,
            leave: body.leave,
            session_pub: self.session_pub().to_vec(),
            intent_seq: self.seq,
            intent_sig: vec![],
        };
        sign_input(&self.key, &mut event);
        event
    }
}

impl Default for IntentSession {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bind_clears_only_after_note_put_ok() {
        let mut s = IntentSession::new();
        assert!(s.bind_pending());
        let ev = s.signed_intent(Some("jwt"), IntentBody::default());
        assert!(!ev.room_token.is_empty());
        assert!(s.bind_pending());
        s.note_bind_put_ok();
        assert!(s.bind_pending());
        s.note_bind_put_ok();
        s.note_bind_put_ok();
        assert!(!s.bind_pending());
        let steady = s.signed_intent(Some("jwt"), IntentBody::default());
        assert!(steady.room_token.is_empty());
    }
}
