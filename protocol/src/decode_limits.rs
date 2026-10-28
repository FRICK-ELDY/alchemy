//! Pre-decode byte limits and post-decode count/string limits (x003b).
//!
//! Guarantees: class-specific ceilings reject oversized Zenoh samples before / just
//! after prost decode so host and client do not allocate unbounded from peer payloads.
//! Non-guarantees: router max-payload / per-peer QoS; cargo-fuzz coverage.

use crate::{
    InputEvent, PresenceState, RenderFrame, SceneManifest, ROOM_ID_MAX_BYTES, USER_ROOT_SLOTS,
};

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
pub const MAX_FRAME_UPSERTS: usize = 4096;
pub const MAX_FRAME_REMOVES: usize = 4096;
pub const MAX_PRESENCE_ASSIGNMENTS: usize = USER_ROOT_SLOTS;

/// Post-decode string / JWT ceilings (bytes of UTF-8 / opaque text).
pub const MAX_PROTOCOL_PIN_BYTES: usize = 128;
pub const MAX_BLAKE3_HEX_BYTES: usize = 64;
pub const MAX_JWT_STRING_BYTES: usize = 16 * 1024;
pub const MAX_PRESENCE_SUB_BYTES: usize = 128;

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

/// Reject before `Message::decode` when the Zenoh sample exceeds the class ceiling.
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

/// Post-decode Input token string ceiling.
pub fn check_input(event: &InputEvent) -> Result<(), DecodeLimitError> {
    check_str("room_token", &event.room_token, MAX_JWT_STRING_BYTES)?;
    Ok(())
}

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
    fn empty_messages_ok() {
        assert!(check_manifest(&SceneManifest::default()).is_ok());
        assert!(check_frame(&RenderFrame::default()).is_ok());
        assert!(check_presence(&PresenceState::default()).is_ok());
        assert!(check_input(&InputEvent::default()).is_ok());
    }
}
