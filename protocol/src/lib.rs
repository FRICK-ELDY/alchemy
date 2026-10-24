//! Alchemy wire contracts (Protobuf).
//!
//! # Guarantees
//! - `PROTOCOL_PIN` identifies the schema generation expected by peers.
//! - `RenderFrame` / `InputEvent` encode/decode via prost.
//!
//! # Non-guarantees
//! - Delta compression, DrawCommand taxonomy, audio/tracking frames.
//! - Cryptographic verification of `InputEvent.room_token` (host-server).

pub use prost::Message;

/// Bump when `proto/` has an incompatible wire change.
pub const PROTOCOL_PIN: &str = "alchemy-protocol/0.2.0-slice-b";

/// Default Zenoh key for local room frames.
pub const FRAME_KEY_LOCAL: &str = "alchemy/dev/room/local/frame";

/// Default Zenoh key for local room inputs.
pub const INPUT_KEY_LOCAL: &str = "alchemy/dev/room/local/input";

/// Build a frame key for an arbitrary room id (ASCII, no `/`).
pub fn frame_key(room_id: &str) -> String {
    format!("alchemy/dev/room/{room_id}/frame")
}

/// Build an input key for an arbitrary room id (ASCII, no `/`).
pub fn input_key(room_id: &str) -> String {
    format!("alchemy/dev/room/{room_id}/input")
}

include!(concat!(env!("OUT_DIR"), "/alchemy.render.rs"));
include!(concat!(env!("OUT_DIR"), "/alchemy.input.rs"));

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn roundtrip_minimal_frame() {
        let frame = RenderFrame {
            tick: 42,
            host_time_ms: 1_000,
            entities: vec![Entity {
                id: 1,
                x: 1.5,
                y: 0.0,
                z: -2.0,
                r: 1.0,
                g: 0.2,
                b: 0.1,
                a: 1.0,
            }],
        };
        let bytes = frame.encode_to_vec();
        let decoded = RenderFrame::decode(bytes.as_slice()).expect("decode");
        assert_eq!(decoded.tick, 42);
        assert_eq!(decoded.entities.len(), 1);
        assert!((decoded.entities[0].x - 1.5).abs() < f64::EPSILON);
    }

    #[test]
    fn roundtrip_input_event() {
        let event = InputEvent {
            room_token: "dummy.jwt".into(),
            client_time_ms: 99,
            move_x: 0.5,
            move_y: -0.25,
            move_z: 0.0,
        };
        let bytes = event.encode_to_vec();
        let decoded = InputEvent::decode(bytes.as_slice()).expect("decode");
        assert_eq!(decoded.room_token, "dummy.jwt");
        assert!((decoded.move_x - 0.5).abs() < f32::EPSILON);
    }

    #[test]
    fn key_helpers() {
        assert_eq!(frame_key("local"), FRAME_KEY_LOCAL);
        assert_eq!(input_key("local"), INPUT_KEY_LOCAL);
    }
}
