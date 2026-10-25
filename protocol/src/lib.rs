//! Alchemy wire contracts (Protobuf).
//!
//! # Guarantees
//! - `PROTOCOL_PIN` identifies the schema generation expected by peers.
//! - `RenderFrame` (delta upserts/removes), `SceneManifest` (+ `LocomotionConfig`), `InputEvent` encode/decode.
//!
//! # Non-guarantees
//! - Bit-packing / prediction coding, DrawCommand taxonomy, audio/tracking frames.
//! - Cryptographic verification of `InputEvent.room_token` (host-server).
//! - Signed Manifest issuance by world-server (later).
//! - Locomotion module list UI / suppress sources.

pub use prost::Message;

/// Bump when `proto/` has an incompatible wire change.
pub const PROTOCOL_PIN: &str = "alchemy-protocol/0.4.0-x0009";

/// Default free-move speed (m/s) when Manifest `move_speed` is 0.
/// Shared by host Noclip and client Settings display (x000a).
pub const DEFAULT_MOVE_SPEED: f32 = 4.0;

/// `InputEvent.buttons` bits (mirror of proto `InputButtons`).
pub mod input_buttons {
    pub const PRIMARY: u32 = 1;
    pub const SECONDARY: u32 = 2;
    pub const GRAB: u32 = 4;
    pub const CONTEXT: u32 = 8;
    pub const JUMP: u32 = 16;
}

/// Default Zenoh key for local room frames (high-frequency pose delta).
pub const FRAME_KEY_LOCAL: &str = "alchemy/dev/room/local/frame";

/// Default Zenoh key for local room inputs.
pub const INPUT_KEY_LOCAL: &str = "alchemy/dev/room/local/input";

/// Default Zenoh key for local room scene manifest (low-frequency blob refs).
pub const MANIFEST_KEY_LOCAL: &str = "alchemy/dev/room/local/manifest";

/// Default Zenoh key for local host control (Slice E stub: shutdown).
pub const HOST_CONTROL_KEY_LOCAL: &str = "alchemy/dev/room/local/host_control";

/// Payload accepted by host-server on `HOST_CONTROL_KEY_LOCAL` (Slice E stub).
pub const HOST_CONTROL_SHUTDOWN: &[u8] = b"shutdown";

/// Build a frame key for an arbitrary room id (ASCII, no `/`).
pub fn frame_key(room_id: &str) -> String {
    format!("alchemy/dev/room/{room_id}/frame")
}

/// Build an input key for an arbitrary room id (ASCII, no `/`).
pub fn input_key(room_id: &str) -> String {
    format!("alchemy/dev/room/{room_id}/input")
}

/// Build a manifest key for an arbitrary room id (ASCII, no `/`).
pub fn manifest_key(room_id: &str) -> String {
    format!("alchemy/dev/room/{room_id}/manifest")
}

/// Build a host-control key for an arbitrary room id (ASCII, no `/`).
pub fn host_control_key(room_id: &str) -> String {
    format!("alchemy/dev/room/{room_id}/host_control")
}

include!(concat!(env!("OUT_DIR"), "/alchemy.render.rs"));
include!(concat!(env!("OUT_DIR"), "/alchemy.input.rs"));
include!(concat!(env!("OUT_DIR"), "/alchemy.scene.rs"));

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn roundtrip_delta_frame() {
        let frame = RenderFrame {
            tick: 42,
            host_time_ms: 1_000,
            upserts: vec![Entity {
                id: 1,
                x: 1.5,
                y: 0.0,
                z: -2.0,
                r: 1.0,
                g: 0.2,
                b: 0.1,
                a: 1.0,
            }],
            removes: vec![9],
        };
        let bytes = frame.encode_to_vec();
        let decoded = RenderFrame::decode(bytes.as_slice()).expect("decode");
        assert_eq!(decoded.tick, 42);
        assert_eq!(decoded.upserts.len(), 1);
        assert!((decoded.upserts[0].x - 1.5).abs() < f64::EPSILON);
        assert_eq!(decoded.removes, vec![9]);
    }

    #[test]
    fn roundtrip_scene_manifest() {
        let manifest = SceneManifest {
            room_id: "local".into(),
            revision: 1,
            entries: vec![SceneEntry {
                entity_id: 1,
                blake3: "fa24848fa7b971718ce6e8dc9bd2d211808fd64e4ddf555feb66d4f82ee1e72a".into(),
                x: 0.0,
                y: 0.0,
                z: 0.0,
            }],
            locomotion: Some(LocomotionConfig {
                mode: LocomotionMode::Noclip.into(),
                move_speed: 4.0,
            }),
        };
        let bytes = manifest.encode_to_vec();
        let decoded = SceneManifest::decode(bytes.as_slice()).expect("decode");
        assert_eq!(decoded.room_id, "local");
        assert_eq!(decoded.revision, 1);
        assert_eq!(decoded.entries.len(), 1);
        assert_eq!(decoded.entries[0].entity_id, 1);
        assert_eq!(
            decoded.entries[0].blake3,
            "fa24848fa7b971718ce6e8dc9bd2d211808fd64e4ddf555feb66d4f82ee1e72a"
        );
        let loco = decoded.locomotion.expect("locomotion");
        assert_eq!(loco.mode, i32::from(LocomotionMode::Noclip));
        assert!((loco.move_speed - 4.0).abs() < f32::EPSILON);
    }

    #[test]
    fn roundtrip_input_event() {
        let event = InputEvent {
            room_token: "dummy.jwt".into(),
            client_time_ms: 99,
            move_x: 0.5,
            move_y: -0.25,
            move_z: 0.0,
            buttons: input_buttons::PRIMARY | input_buttons::JUMP,
        };
        let bytes = event.encode_to_vec();
        let decoded = InputEvent::decode(bytes.as_slice()).expect("decode");
        assert_eq!(decoded.room_token, "dummy.jwt");
        assert!((decoded.move_x - 0.5).abs() < f32::EPSILON);
        assert_eq!(
            decoded.buttons,
            input_buttons::PRIMARY | input_buttons::JUMP
        );
    }

    #[test]
    fn key_helpers() {
        assert_eq!(frame_key("local"), FRAME_KEY_LOCAL);
        assert_eq!(input_key("local"), INPUT_KEY_LOCAL);
        assert_eq!(manifest_key("local"), MANIFEST_KEY_LOCAL);
        assert_eq!(host_control_key("local"), HOST_CONTROL_KEY_LOCAL);
    }
}
