//! Alchemy wire contracts (Protobuf).
//!
//! # Guarantees
//! - `PROTOCOL_PIN` identifies the schema generation expected by peers.
//! - `RenderFrame` encodes/decodes via prost as the Slice A frame payload.
//!
//! # Non-guarantees
//! - Delta compression, DrawCommand taxonomy, audio/tracking frames.
//! - Room Token / ACL fields on the wire (deferred to world-server).

pub use prost::Message;

/// Bump when `proto/` has an incompatible wire change.
pub const PROTOCOL_PIN: &str = "alchemy-protocol/0.1.0-slice-a";

/// Default Zenoh key for local Slice A room frames.
pub const FRAME_KEY_LOCAL: &str = "alchemy/dev/room/local/frame";

/// Build a frame key for an arbitrary room id (ASCII, no `/`).
pub fn frame_key(room_id: &str) -> String {
    format!("alchemy/dev/room/{room_id}/frame")
}

include!(concat!(env!("OUT_DIR"), "/alchemy.render.rs"));

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
}
