//! Client Intent session key (x0023).
//!
//! Guarantees: bind Intent carries Room Token until enough successful puts; steady
//! Intents are Ed25519-signed thereafter.
//! Non-guarantees: host acceptance ack (Zenoh put ≠ verify); key persistence across restart.

use alchemy_protocol::wire_crypto::{
    generate_signing_key, public_key_bytes, sign_input, SigningKey,
};
use alchemy_protocol::InputEvent;

/// Successful Zenoh puts that must still include `room_token` after join / refresh.
/// Covers put failure retries and brief host reject windows without an ack channel.
const BIND_PUTS_REQUIRED: u32 = 3;

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

    /// Call only after Zenoh `put` succeeded for an Intent that included `room_token`.
    pub fn note_bind_put_ok(&mut self) {
        self.bind_puts_remaining = self.bind_puts_remaining.saturating_sub(1);
    }

    /// Build a signed Intent. Does **not** clear bind state (caller does after put OK).
    pub fn signed_intent(
        &mut self,
        room_token: Option<&str>,
        client_time_ms: u64,
        move_x: f32,
        move_y: f32,
        move_z: f32,
        buttons: u32,
        grab_entity_id: u32,
        aim_x: f32,
        aim_y: f32,
        aim_z: f32,
        leave: bool,
    ) -> InputEvent {
        self.seq = self.seq.saturating_add(1);
        let include_token = self.bind_puts_remaining > 0;
        let token = if include_token {
            room_token.unwrap_or("").to_string()
        } else {
            String::new()
        };
        let mut event = InputEvent {
            room_token: token,
            client_time_ms,
            move_x,
            move_y,
            move_z,
            buttons,
            grab_entity_id,
            aim_x,
            aim_y,
            aim_z,
            leave,
            session_pub: public_key_bytes(&self.key),
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
        let ev = s.signed_intent(Some("jwt"), 0, 0.0, 0.0, 0.0, 0, 0, 0.0, 0.0, 0.0, false);
        assert!(!ev.room_token.is_empty());
        assert!(s.bind_pending());
        s.note_bind_put_ok();
        assert!(s.bind_pending());
        s.note_bind_put_ok();
        s.note_bind_put_ok();
        assert!(!s.bind_pending());
        let steady = s.signed_intent(Some("jwt"), 0, 0.0, 0.0, 0.0, 0, 0, 0.0, 0.0, 0.0, false);
        assert!(steady.room_token.is_empty());
    }
}
