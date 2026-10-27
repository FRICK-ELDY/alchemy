//! Role → capability mapping (x002a / D4).
//!
//! Guarantees: Room Token `role` maps to a bitset; Intent path uses INTENT_* bits;
//! world HTTP uses HTTP_* bits (enforced by world-server via `role == "host"`,
//! matching this table — Elixir cannot import this crate).
//! Non-guarantees: caps on Presence wire; JWT `caps` claim; shared-sim / invite ACL;
//! mid-session role change without re-bind (caps freeze at Intent bind).
//!
//! PROTOCOL_PIN is unchanged — this is not a protobuf schema change.

use crate::input_buttons;
use crate::InputEvent;

/// Apply move Intent to Presence overlay / Rapier.
pub const INTENT_MOVE: u32 = 1 << 0;
/// Grab button + aim / grab_entity_id.
pub const INTENT_GRAB: u32 = 1 << 1;
/// Explicit leave → Presence reclaim.
pub const INTENT_LEAVE: u32 = 1 << 2;

/// `GET …/scene` (world HTTP; host Room Token).
pub const HTTP_SCENE: u32 = 1 << 8;
/// `POST …/host_keys` HostAttestation.
pub const HTTP_ATTEST: u32 = 1 << 9;
/// `POST …/manifest_seal`.
pub const HTTP_SEAL: u32 = 1 << 10;

const INTENT_MASK: u32 = INTENT_MOVE | INTENT_GRAB | INTENT_LEAVE;

/// Full capability bitset for a Room Token `role`.
pub fn caps_for_role(role: &str) -> u32 {
    match role {
        "guest" => INTENT_MOVE | INTENT_GRAB | INTENT_LEAVE,
        "host" => INTENT_MOVE | INTENT_GRAB | INTENT_LEAVE | HTTP_SCENE | HTTP_ATTEST | HTTP_SEAL,
        _ => 0,
    }
}

/// Intent-path subset stored on host SessionBinding.
pub fn intent_caps_for_role(role: &str) -> u32 {
    caps_for_role(role) & INTENT_MASK
}

/// True when every bit in `required` is set in `caps`.
pub fn has_all(caps: u32, required: u32) -> bool {
    caps & required == required
}

/// True when any bit in `bits` is set in `caps`.
pub fn has_any(caps: u32, bits: u32) -> bool {
    caps & bits != 0
}

/// Zero / clear Intent fields the session is not allowed to exercise.
pub fn mask_intent(caps: u32, event: &mut InputEvent) {
    if !has_all(caps, INTENT_MOVE) {
        event.move_x = 0.0;
        event.move_y = 0.0;
        event.move_z = 0.0;
    }
    if !has_all(caps, INTENT_GRAB) {
        event.buttons &= !input_buttons::GRAB;
        event.grab_entity_id = 0;
        event.aim_x = 0.0;
        event.aim_y = 0.0;
        event.aim_z = 0.0;
    }
    if !has_all(caps, INTENT_LEAVE) {
        event.leave = false;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn guest_has_intent_not_http() {
        let c = caps_for_role("guest");
        assert!(has_all(c, INTENT_MOVE | INTENT_GRAB | INTENT_LEAVE));
        assert!(!has_any(c, HTTP_SCENE | HTTP_ATTEST | HTTP_SEAL));
        assert_eq!(intent_caps_for_role("guest"), c);
        assert_eq!(c, INTENT_MOVE | INTENT_GRAB | INTENT_LEAVE);
    }

    #[test]
    fn host_has_intent_and_http() {
        let c = caps_for_role("host");
        assert!(has_all(c, INTENT_MOVE | INTENT_GRAB | INTENT_LEAVE));
        assert!(has_all(c, HTTP_SCENE | HTTP_ATTEST | HTTP_SEAL));
        assert_eq!(
            intent_caps_for_role("host"),
            INTENT_MOVE | INTENT_GRAB | INTENT_LEAVE
        );
    }

    #[test]
    fn unknown_role_is_empty() {
        assert_eq!(caps_for_role("admin"), 0);
        assert_eq!(intent_caps_for_role(""), 0);
    }

    #[test]
    fn mask_clears_disallowed_fields() {
        let mut event = InputEvent {
            room_token: String::new(),
            client_time_ms: 0,
            move_x: 1.0,
            move_y: 0.5,
            move_z: -1.0,
            buttons: input_buttons::GRAB | input_buttons::PRIMARY,
            grab_entity_id: 14,
            aim_x: 0.0,
            aim_y: 1.0,
            aim_z: 0.0,
            leave: true,
            session_pub: vec![],
            intent_seq: 1,
            intent_sig: vec![],
        };
        mask_intent(INTENT_MOVE, &mut event);
        assert!((event.move_x - 1.0).abs() < f32::EPSILON);
        assert_eq!(event.buttons & input_buttons::GRAB, 0);
        assert_eq!(
            event.buttons & input_buttons::PRIMARY,
            input_buttons::PRIMARY
        );
        assert_eq!(event.grab_entity_id, 0);
        assert!(!event.leave);

        event.leave = true;
        mask_intent(INTENT_LEAVE, &mut event);
        assert!(event.leave);
        assert!((event.move_x).abs() < f32::EPSILON);
    }
}
