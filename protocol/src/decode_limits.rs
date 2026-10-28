//! Pre-decode byte limits and post-decode count/string limits (x003b).
//!
//! Guarantees: class-specific ceilings reject oversized Zenoh samples before / just
//! after prost decode so host and client do not allocate unbounded from peer payloads.
//! Callers should use [`ZBytes::len`](https://docs.rs/zenoh/latest/zenoh/bytes/struct.ZBytes.html)
//! (or equivalent) **before** `to_bytes()` / contiguous copy when the transport exposes a length.
//!
//! Non-guarantees: router max-payload / per-peer QoS; cargo-fuzz coverage.
//!
//! # Frame upsert / remove ceilings
//! [`MAX_FRAME_UPSERTS`] / [`MAX_FRAME_REMOVES`] are **DoS bounds**, not the host's steady
//! publish size. Demo host scenes stay far below. If host publish budgets grow toward these
//! limits, raise the constants and host-side guards together so clients do not silently
//! drop every frame.

use prost::Message;

use crate::wire_crypto::{PUBLIC_KEY_LENGTH, SIGNATURE_LENGTH};
use crate::{
    InputEvent, PresenceState, RenderFrame, SceneManifest, ROOM_ID_MAX_BYTES, USER_ROOT_SLOTS,
};

/// Ed25519 signature wire length ceiling (aligned with [`SIGNATURE_LENGTH`]).
pub const MAX_INTENT_SIG_BYTES: usize = SIGNATURE_LENGTH;

/// Wire message class for pre-decode sizing.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WireClass {
    Manifest,
    Frame,
    Presence,
    Input,
}

impl WireClass {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Manifest => "manifest",
            Self::Frame => "frame",
            Self::Presence => "presence",
            Self::Input => "input",
        }
    }
}

/// Pre-decode payload byte ceilings (inclusive).
pub const MAX_MANIFEST_BYTES: usize = 1024 * 1024;
pub const MAX_FRAME_BYTES: usize = 256 * 1024;
pub const MAX_PRESENCE_BYTES: usize = 64 * 1024;
pub const MAX_INPUT_BYTES: usize = 32 * 1024;

/// Post-decode repeated-field ceilings.
pub const MAX_MANIFEST_ENTRIES: usize = 4096;
/// DoS bound — see module docs (not a host steady-state publish quota).
pub const MAX_FRAME_UPSERTS: usize = 4096;
pub const MAX_FRAME_REMOVES: usize = 4096;
pub const MAX_PRESENCE_ASSIGNMENTS: usize = USER_ROOT_SLOTS;

/// Post-decode string / JWT / key ceilings (bytes).
pub const MAX_PROTOCOL_PIN_BYTES: usize = 128;
pub const MAX_BLAKE3_HEX_BYTES: usize = 64;
pub const MAX_JWT_STRING_BYTES: usize = 16 * 1024;
pub const MAX_PRESENCE_SUB_BYTES: usize = 128;
/// `session_pub` must be empty (pre-bind omit) or ≤ Ed25519 pub length.
pub const MAX_SESSION_PUB_BYTES: usize = PUBLIC_KEY_LENGTH;

/// Why a sample was rejected by decode limits.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DecodeLimitError {
    PayloadTooLarge {
        class: WireClass,
        len: usize,
        max: usize,
    },
    TooManyEntries {
        field: &'static str,
        count: usize,
        max: usize,
    },
    StringTooLong {
        field: &'static str,
        len: usize,
        max: usize,
    },
    BytesTooLong {
        field: &'static str,
        len: usize,
        max: usize,
    },
}

impl std::fmt::Display for DecodeLimitError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::PayloadTooLarge { class, len, max } => {
                write!(
                    f,
                    "{} payload too large: {len} > {max} bytes",
                    class.as_str()
                )
            }
            Self::TooManyEntries { field, count, max } => {
                write!(f, "too many {field}: {count} > {max}")
            }
            Self::StringTooLong { field, len, max } => {
                write!(f, "{field} too long: {len} > {max} bytes")
            }
            Self::BytesTooLong { field, len, max } => {
                write!(f, "{field} too long: {len} > {max} bytes")
            }
        }
    }
}

impl std::error::Error for DecodeLimitError {}

/// Max payload bytes for `class` (inclusive).
pub const fn max_payload_bytes(class: WireClass) -> usize {
    match class {
        WireClass::Manifest => MAX_MANIFEST_BYTES,
        WireClass::Frame => MAX_FRAME_BYTES,
        WireClass::Presence => MAX_PRESENCE_BYTES,
        WireClass::Input => MAX_INPUT_BYTES,
    }
}

/// Reject before contiguous copy / `Message::decode` when the sample length exceeds the ceiling.
pub fn check_payload_bytes(class: WireClass, len: usize) -> Result<(), DecodeLimitError> {
    let max = max_payload_bytes(class);
    if len > max {
        Err(DecodeLimitError::PayloadTooLarge { class, len, max })
    } else {
        Ok(())
    }
}

fn check_str(field: &'static str, value: &str, max: usize) -> Result<(), DecodeLimitError> {
    let len = value.len();
    if len > max {
        Err(DecodeLimitError::StringTooLong { field, len, max })
    } else {
        Ok(())
    }
}

fn check_bytes(field: &'static str, value: &[u8], max: usize) -> Result<(), DecodeLimitError> {
    let len = value.len();
    if len > max {
        Err(DecodeLimitError::BytesTooLong { field, len, max })
    } else {
        Ok(())
    }
}

fn check_count(field: &'static str, count: usize, max: usize) -> Result<(), DecodeLimitError> {
    if count > max {
        Err(DecodeLimitError::TooManyEntries { field, count, max })
    } else {
        Ok(())
    }
}

/// Post-decode Manifest structural limits (entries + strings).
pub fn check_manifest(manifest: &SceneManifest) -> Result<(), DecodeLimitError> {
    check_str("room_id", &manifest.room_id, ROOM_ID_MAX_BYTES)?;
    check_str(
        "protocol_pin",
        &manifest.protocol_pin,
        MAX_PROTOCOL_PIN_BYTES,
    )?;
    check_str(
        "host_attestation",
        &manifest.host_attestation,
        MAX_JWT_STRING_BYTES,
    )?;
    check_str("world_seal", &manifest.world_seal, MAX_JWT_STRING_BYTES)?;
    check_count("entries", manifest.entries.len(), MAX_MANIFEST_ENTRIES)?;
    for entry in &manifest.entries {
        check_str("entry.blake3", &entry.blake3, MAX_BLAKE3_HEX_BYTES)?;
    }
    Ok(())
}

/// Post-decode Frame upserts / removes ceilings.
pub fn check_frame(frame: &RenderFrame) -> Result<(), DecodeLimitError> {
    check_count("upserts", frame.upserts.len(), MAX_FRAME_UPSERTS)?;
    check_count("removes", frame.removes.len(), MAX_FRAME_REMOVES)?;
    Ok(())
}

/// Post-decode Presence assignment / string ceilings.
pub fn check_presence(state: &PresenceState) -> Result<(), DecodeLimitError> {
    check_str("protocol_pin", &state.protocol_pin, MAX_PROTOCOL_PIN_BYTES)?;
    check_count(
        "assignments",
        state.assignments.len(),
        MAX_PRESENCE_ASSIGNMENTS,
    )?;
    for a in &state.assignments {
        check_str("assignment.sub", &a.sub, MAX_PRESENCE_SUB_BYTES)?;
    }
    Ok(())
}

/// Post-decode Input token / session key ceilings (before host authorize).
pub fn check_input(event: &InputEvent) -> Result<(), DecodeLimitError> {
    check_str("room_token", &event.room_token, MAX_JWT_STRING_BYTES)?;
    check_bytes("session_pub", &event.session_pub, MAX_SESSION_PUB_BYTES)?;
    check_bytes("intent_sig", &event.intent_sig, MAX_INTENT_SIG_BYTES)?;
    Ok(())
}

/// Payload byte gate → decode → post-decode count check (wiring / regression helper).
pub fn decode_frame_checked(payload: &[u8]) -> Result<RenderFrame, FrameDecodeLimitError> {
    check_payload_bytes(WireClass::Frame, payload.len()).map_err(FrameDecodeLimitError::Limit)?;
    let frame = RenderFrame::decode(payload).map_err(FrameDecodeLimitError::Decode)?;
    check_frame(&frame).map_err(FrameDecodeLimitError::Limit)?;
    Ok(frame)
}

/// Decode + post-check Input after payload byte gate.
pub fn decode_input_checked(payload: &[u8]) -> Result<InputEvent, InputDecodeLimitError> {
    check_payload_bytes(WireClass::Input, payload.len()).map_err(InputDecodeLimitError::Limit)?;
    let event = InputEvent::decode(payload).map_err(InputDecodeLimitError::Decode)?;
    check_input(&event).map_err(InputDecodeLimitError::Limit)?;
    Ok(event)
}

/// Errors from [`decode_frame_checked`].
#[derive(Debug)]
pub enum FrameDecodeLimitError {
    Limit(DecodeLimitError),
    Decode(prost::DecodeError),
}

impl std::fmt::Display for FrameDecodeLimitError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Limit(e) => write!(f, "{e}"),
            Self::Decode(e) => write!(f, "decode: {e}"),
        }
    }
}

impl std::error::Error for FrameDecodeLimitError {}

/// Errors from [`decode_input_checked`].
#[derive(Debug)]
pub enum InputDecodeLimitError {
    Limit(DecodeLimitError),
    Decode(prost::DecodeError),
}

impl std::fmt::Display for InputDecodeLimitError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Limit(e) => write!(f, "{e}"),
            Self::Decode(e) => write!(f, "decode: {e}"),
        }
    }
}

impl std::error::Error for InputDecodeLimitError {}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{PresenceAssignment, SceneEntry};

    #[test]
    fn payload_byte_gate() {
        assert!(check_payload_bytes(WireClass::Frame, MAX_FRAME_BYTES).is_ok());
        assert!(matches!(
            check_payload_bytes(WireClass::Frame, MAX_FRAME_BYTES + 1),
            Err(DecodeLimitError::PayloadTooLarge { .. })
        ));
        assert!(matches!(
            check_payload_bytes(WireClass::Manifest, MAX_MANIFEST_BYTES + 1),
            Err(DecodeLimitError::PayloadTooLarge { .. })
        ));
        assert!(matches!(
            check_payload_bytes(WireClass::Input, MAX_INPUT_BYTES + 1),
            Err(DecodeLimitError::PayloadTooLarge { .. })
        ));
        assert!(matches!(
            check_payload_bytes(WireClass::Presence, MAX_PRESENCE_BYTES + 1),
            Err(DecodeLimitError::PayloadTooLarge { .. })
        ));
    }

    #[test]
    fn decode_frame_checked_rejects_oversize_before_decode() {
        let junk = vec![0u8; MAX_FRAME_BYTES + 1];
        assert!(matches!(
            decode_frame_checked(&junk),
            Err(FrameDecodeLimitError::Limit(DecodeLimitError::PayloadTooLarge { .. }))
        ));
    }

    #[test]
    fn decode_input_checked_rejects_oversize_session_pub() {
        let event = InputEvent {
            session_pub: vec![0u8; MAX_SESSION_PUB_BYTES + 1],
            ..Default::default()
        };
        let bytes = event.encode_to_vec();
        assert!(
            bytes.len() <= MAX_INPUT_BYTES,
            "test fixture must fit pre-decode byte gate"
        );
        assert!(matches!(
            decode_input_checked(&bytes),
            Err(InputDecodeLimitError::Limit(DecodeLimitError::BytesTooLong {
                field: "session_pub",
                ..
            }))
        ));
    }

    #[test]
    fn manifest_rejects_too_many_entries() {
        let manifest = SceneManifest {
            entries: (0..=MAX_MANIFEST_ENTRIES)
                .map(|i| SceneEntry {
                    entity_id: i as u32,
                    ..Default::default()
                })
                .collect(),
            ..Default::default()
        };
        assert!(matches!(
            check_manifest(&manifest),
            Err(DecodeLimitError::TooManyEntries {
                field: "entries",
                ..
            })
        ));
    }

    #[test]
    fn manifest_rejects_long_blake3() {
        let manifest = SceneManifest {
            entries: vec![SceneEntry {
                blake3: "a".repeat(MAX_BLAKE3_HEX_BYTES + 1),
                ..Default::default()
            }],
            ..Default::default()
        };
        assert!(matches!(
            check_manifest(&manifest),
            Err(DecodeLimitError::StringTooLong {
                field: "entry.blake3",
                ..
            })
        ));
    }

    #[test]
    fn frame_rejects_too_many_upserts() {
        let frame = RenderFrame {
            upserts: (0..=MAX_FRAME_UPSERTS)
                .map(|i| crate::Entity {
                    id: i as u32,
                    ..Default::default()
                })
                .collect(),
            ..Default::default()
        };
        assert!(matches!(
            check_frame(&frame),
            Err(DecodeLimitError::TooManyEntries {
                field: "upserts",
                ..
            })
        ));
    }

    #[test]
    fn frame_rejects_too_many_removes() {
        let frame = RenderFrame {
            removes: (0..=MAX_FRAME_REMOVES as u32).collect(),
            ..Default::default()
        };
        assert!(matches!(
            check_frame(&frame),
            Err(DecodeLimitError::TooManyEntries {
                field: "removes",
                ..
            })
        ));
    }

    #[test]
    fn presence_rejects_too_many_assignments() {
        let state = PresenceState {
            assignments: (0..=MAX_PRESENCE_ASSIGNMENTS)
                .map(|i| PresenceAssignment {
                    sub: format!("user-{i}"),
                    entity_id: 3 + i as u32,
                })
                .collect(),
            ..Default::default()
        };
        assert!(matches!(
            check_presence(&state),
            Err(DecodeLimitError::TooManyEntries {
                field: "assignments",
                ..
            })
        ));
    }

    #[test]
    fn input_rejects_long_room_token() {
        let event = InputEvent {
            room_token: "x".repeat(MAX_JWT_STRING_BYTES + 1),
            ..Default::default()
        };
        assert!(matches!(
            check_input(&event),
            Err(DecodeLimitError::StringTooLong {
                field: "room_token",
                ..
            })
        ));
    }

    #[test]
    fn input_rejects_long_intent_sig() {
        let event = InputEvent {
            intent_sig: vec![0u8; MAX_INTENT_SIG_BYTES + 1],
            ..Default::default()
        };
        assert!(matches!(
            check_input(&event),
            Err(DecodeLimitError::BytesTooLong {
                field: "intent_sig",
                ..
            })
        ));
    }

    #[test]
    fn empty_messages_ok() {
        assert!(check_manifest(&SceneManifest::default()).is_ok());
        assert!(check_frame(&RenderFrame::default()).is_ok());
        assert!(check_presence(&PresenceState::default()).is_ok());
        assert!(check_input(&InputEvent::default()).is_ok());
    }
}
