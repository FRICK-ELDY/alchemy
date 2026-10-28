//! Alchemy wire contracts (Protobuf).
//!
//! # Guarantees
//! - `PROTOCOL_PIN` identifies the schema generation expected by peers (also on Manifest/Presence).
//! - `RenderFrame` (delta upserts/removes + host_sig), `SceneManifest` (+ `LocomotionConfig`, host_pub),
//!   `InputEvent` (session Ed25519), `PresenceState` (`sub`→entity) encode/decode.
//! - [`wire_crypto`] sign/verify helpers for Intent / Frame (x0023).
//! - [`sim_capability`] role → Intent/HTTP capability bits (x002a; not a wire schema).
//! - [`generation_gate`] monotonic `(host_generation, revision)` (x003a).
//! - [`decode_limits`] class-specific pre/post decode ceilings (x003b; **does not bump** pin —
//!   local validation, wire-compatible with x003a peers).
//! - Delivery class table: [`peer-delivery.md`](../peer-delivery.md). Frame tick gap → snapshot
//!   ([`decide_frame_tick`], [`FRAME_TICK_GAP_RESYNC`]; x0024).
//!
//! # Non-guarantees
//! - Bit-packing / prediction coding, DrawCommand taxonomy, audio/tracking frames.
//! - Cryptographic verification of `InputEvent.room_token` (host-server JWKS).
//! - Rapier / Formula execution (see `alchemy-shared-sim` EntityKind index; x0030).
//! - Frame scale animation / non-uniform collider resize (x0029 emits unit scale).
//! - Caps on Presence / JWT claim; invite ACL; reliable-delivery ACK / event-id channel.
//! - Manifest scale resizing Rapier colliders (half_extent stays definition size).
//! - Locomotion module list UI / suppress sources.
//! - Tenant namespace `alchemy/tenant/...` (dev keys only: `alchemy/dev/room/{room_id}/…`).
//! - 5+ concurrent guests / WAN liveliness ACL / Zenoh ACL.
//! - Full host handoff UX / Wake-on-Join (generation advance is the contract only).
//! - Router max-payload / per-peer QoS (app-level decode limits only; x003b).

pub use prost::Message;

pub mod decode_limits;
pub mod generation_gate;
pub mod host_ipc;
pub mod sim_capability;
pub mod wire_crypto;

/// Bump when `proto/` has an incompatible wire change.
///
/// x003b decode limits are local validation only — pin stays on x003a.
pub const PROTOCOL_PIN: &str = "alchemy-protocol/0.16.0-x003a";

/// Identity orientation on the wire (`Entity.qx..qw`, xyzw).
pub const ENTITY_QUAT_IDENTITY: [f64; 4] = [0.0, 0.0, 0.0, 1.0];

/// Unit scale on the wire (`Entity.sx..sz`).
pub const ENTITY_SCALE_ONE: [f64; 3] = [1.0, 1.0, 1.0];

/// `SceneEntry.flags`: entity may be Grab-held (x0027).
pub const SCENE_FLAG_GRABBABLE: u32 = 1;

/// Default free-move speed (m/s) when Manifest `move_speed` is 0.
/// Shared by host Noclip and client Settings display (x000a).
pub const DEFAULT_MOVE_SPEED: f32 = 4.0;

/// Max simultaneous players (Presence slots / user_roots).
///
/// Shared by host-server / client / e2e-probe to avoid id drift (x001f).
/// Not a wire-schema field — host/content knowledge temporarily parked here;
/// prefer a dedicated content/shared-sim crate if Presence capacity grows.
pub const USER_ROOT_SLOTS: usize = 4;

/// First user_root entity id; last = [`USER_ROOT_ENTITY_FIRST`] + [`USER_ROOT_SLOTS`] - 1.
pub const USER_ROOT_ENTITY_FIRST: u32 = 3;

/// Inclusive last user_root entity id.
pub const fn user_root_entity_last() -> u32 {
    USER_ROOT_ENTITY_FIRST + USER_ROOT_SLOTS as u32 - 1
}

/// True when `flags` includes [`SCENE_FLAG_GRABBABLE`].
pub const fn scene_entry_grabbable(flags: u32) -> bool {
    flags & SCENE_FLAG_GRABBABLE != 0
}

/// True when `entity_id` is in the Presence user_root band.
pub const fn scene_entry_is_user_root(entity_id: u32) -> bool {
    entity_id >= USER_ROOT_ENTITY_FIRST && entity_id <= user_root_entity_last()
}

/// Cuboid draw/collision candidate (x0027).
///
/// Classification priority (host + client must match):
/// 1. user_root id band → Presence spawn (not a box)
/// 2. `blake3` non-empty → mesh only (`half_extent` / Grab flags ignored for box path)
/// 3. `half_extent > 0` → cuboid; Grab only when [`scene_entry_grabbable`]
pub fn scene_entry_is_box(blake3: &str, half_extent: f32, entity_id: u32) -> bool {
    !scene_entry_is_user_root(entity_id) && blake3.is_empty() && half_extent > 0.0
}

/// Build an [`Entity`] with identity orientation and unit scale (x0029).
#[allow(clippy::too_many_arguments)]
pub fn entity_at(id: u32, x: f64, y: f64, z: f64, r: f32, g: f32, b: f32, a: f32) -> Entity {
    entity_trs(
        id,
        x,
        y,
        z,
        ENTITY_QUAT_IDENTITY,
        ENTITY_SCALE_ONE,
        r,
        g,
        b,
        a,
    )
}

/// Build an [`Entity`] with explicit TRS (x0029).
#[allow(clippy::too_many_arguments)]
pub fn entity_trs(
    id: u32,
    x: f64,
    y: f64,
    z: f64,
    quat_xyzw: [f64; 4],
    scale: [f64; 3],
    r: f32,
    g: f32,
    b: f32,
    a: f32,
) -> Entity {
    Entity {
        id,
        x,
        y,
        z,
        r,
        g,
        b,
        a,
        qx: quat_xyzw[0],
        qy: quat_xyzw[1],
        qz: quat_xyzw[2],
        qw: quat_xyzw[3],
        sx: scale[0],
        sy: scale[1],
        sz: scale[2],
    }
}

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

/// Default Zenoh key for local room presence (low-frequency sub→entity; x0015).
pub const PRESENCE_KEY_LOCAL: &str = "alchemy/dev/room/local/presence";

/// Default Zenoh key for local room snapshot queryable (x0023).
pub const SNAPSHOT_KEY_LOCAL: &str = "alchemy/dev/room/local/snapshot";

/// Payload accepted by host-server on `host_control_key(room_id)` (Slice E stub).
pub const HOST_CONTROL_SHUTDOWN: &[u8] = b"shutdown";

/// Env var that opts into unauthenticated Zenoh `host_control` (x001e; default off).
pub const HOST_CONTROL_ZENOH_ENV: &str = "ALCHEMY_HOST_CONTROL";

/// Parse whether Zenoh host_control is enabled from an env value (testable).
///
/// Accepted truthy: `1`, `true`, `TRUE`, `yes`, `YES`. Anything else (incl. unset) is off.
pub fn host_control_zenoh_enabled_from(value: Option<&str>) -> bool {
    matches!(
        value,
        Some("1") | Some("true") | Some("TRUE") | Some("yes") | Some("YES")
    )
}

/// Read [`HOST_CONTROL_ZENOH_ENV`]; default **false** so WAN peers cannot stop host.
pub fn host_control_zenoh_enabled() -> bool {
    host_control_zenoh_enabled_from(std::env::var(HOST_CONTROL_ZENOH_ENV).ok().as_deref())
}

/// Why [`validate_room_id`] rejected a value.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RoomIdError {
    Empty,
    ContainsSlash,
    TooLong,
    InvalidCharset,
}

impl std::fmt::Display for RoomIdError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Empty => write!(f, "room_id must not be empty"),
            Self::ContainsSlash => write!(f, "room_id must not contain '/'"),
            Self::TooLong => write!(f, "room_id must be at most 64 bytes"),
            Self::InvalidCharset => {
                write!(f, "room_id must match [A-Za-z0-9_-]+")
            }
        }
    }
}

impl std::error::Error for RoomIdError {}

/// Max byte length for [`validate_room_id`] (x002e; shared with world).
pub const ROOM_ID_MAX_BYTES: usize = 64;

/// Validate `ROOM_ID` for Zenoh key segments / world Room Token (x0017 / x002e).
///
/// Guarantees: non-empty, no `/`, ≤ [`ROOM_ID_MAX_BYTES`], charset `[A-Za-z0-9_-]`.
/// Non-guarantees: URL encoding, Unicode, uniqueness.
pub fn validate_room_id(room_id: &str) -> Result<&str, RoomIdError> {
    if room_id.is_empty() {
        return Err(RoomIdError::Empty);
    }
    // Slash is also outside the charset below; keep a dedicated error so callers
    // (and tests) can distinguish Zenoh key-segment breaks from other bad chars.
    if room_id.contains('/') {
        return Err(RoomIdError::ContainsSlash);
    }
    if room_id.len() > ROOM_ID_MAX_BYTES {
        return Err(RoomIdError::TooLong);
    }
    if !room_id
        .bytes()
        .all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'-')
    {
        return Err(RoomIdError::InvalidCharset);
    }
    Ok(room_id)
}

/// Build a frame key for an arbitrary room id (caller should [`validate_room_id`]).
pub fn frame_key(room_id: &str) -> String {
    format!("alchemy/dev/room/{room_id}/frame")
}

/// Build an input key for an arbitrary room id (caller should [`validate_room_id`]).
pub fn input_key(room_id: &str) -> String {
    format!("alchemy/dev/room/{room_id}/input")
}

/// Build a manifest key for an arbitrary room id (caller should [`validate_room_id`]).
pub fn manifest_key(room_id: &str) -> String {
    format!("alchemy/dev/room/{room_id}/manifest")
}

/// Build a host-control key for an arbitrary room id (caller should [`validate_room_id`]).
pub fn host_control_key(room_id: &str) -> String {
    format!("alchemy/dev/room/{room_id}/host_control")
}

/// Build a presence key for an arbitrary room id (caller should [`validate_room_id`]).
pub fn presence_key(room_id: &str) -> String {
    format!("alchemy/dev/room/{room_id}/presence")
}

/// Build a snapshot queryable key for an arbitrary room id (x0023).
pub fn snapshot_key(room_id: &str) -> String {
    format!("alchemy/dev/room/{room_id}/snapshot")
}

/// True when wire `protocol_pin` matches this crate's [`PROTOCOL_PIN`].
pub fn protocol_pin_matches(pin: &str) -> bool {
    pin == PROTOCOL_PIN
}

/// Max tick advance allowed on a **delta** `frame` without treating local state as unknown (x0024).
///
/// `1` = consecutive only (`last + 1`). Larger values tolerate small drops under latest semantics
/// but risk ghost entities from missed `removes`. See [`peer-delivery.md`](../peer-delivery.md).
pub const FRAME_TICK_GAP_RESYNC: u64 = 1;

/// What a client should do with a delta `RenderFrame.tick` relative to `last_frame_tick` (x0024).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FrameTickDecision {
    /// Apply the delta (`upserts` / `removes`).
    Apply,
    /// Stale or duplicate tick — drop without resync.
    DropStale,
    /// Tick jumped past [`FRAME_TICK_GAP_RESYNC`]; do **not** apply; fetch `…/snapshot`.
    ResyncSnapshot,
}

/// Decide delta-frame tick handling (snapshot path skips this and replaces entities).
///
/// `max_gap` is normally [`FRAME_TICK_GAP_RESYNC`].
///
/// Monotonic forward only: `tick <= prev` → [`FrameTickDecision::DropStale`].
/// Host uses `wrapping_add` after each **publish**, but u64 wrap is not handled here
/// (would look like a huge rewind → DropStale until Manifest/snapshot resets `last`).
pub fn decide_frame_tick(last: Option<u64>, tick: u64, max_gap: u64) -> FrameTickDecision {
    let Some(prev) = last else {
        return FrameTickDecision::Apply;
    };
    if tick <= prev {
        return FrameTickDecision::DropStale;
    }
    let advance = tick - prev;
    if advance <= max_gap {
        FrameTickDecision::Apply
    } else {
        FrameTickDecision::ResyncSnapshot
    }
}

/// Default Zenoh liveliness key prefix for local room (x0019; `sub` appended).
pub const LIVELINESS_KEY_LOCAL_PREFIX: &str = "alchemy/dev/room/local/alive";

/// Build a liveliness token key bound to Room Token `jti` (x0022).
///
/// `sub` / `jti` must not contain `/`. Host reclaims on Delete only when `jti` matches
/// the last verified Input for that `sub`.
pub fn liveliness_key(room_id: &str, sub: &str, jti: &str) -> String {
    format!("alchemy/dev/room/{room_id}/alive/{sub}/{jti}")
}

/// Host wildcard subscribe key for room liveliness (x0019 / x0022).
pub fn liveliness_wildcard(room_id: &str) -> String {
    format!("alchemy/dev/room/{room_id}/alive/**")
}

/// Parse `(sub, jti)` from `…/alive/{sub}/{jti}`. Returns `None` if shape mismatches.
pub fn liveliness_binding_from_key(key: &str) -> Option<(&str, &str)> {
    let (_, rest) = key.split_once("/alive/")?;
    let (sub, jti) = rest.split_once('/')?;
    if sub.is_empty() || jti.is_empty() || jti.contains('/') {
        return None;
    }
    Some((sub, jti))
}

/// Deprecated shape helper — x0019 single-segment keys are rejected (x0022).
#[deprecated(note = "use liveliness_binding_from_key")]
pub fn liveliness_sub_from_key(key: &str) -> Option<&str> {
    liveliness_binding_from_key(key).map(|(sub, _)| sub)
}

include!(concat!(env!("OUT_DIR"), "/alchemy.render.rs"));
include!(concat!(env!("OUT_DIR"), "/alchemy.input.rs"));
include!(concat!(env!("OUT_DIR"), "/alchemy.scene.rs"));
include!(concat!(env!("OUT_DIR"), "/alchemy.presence.rs"));

/// Normalized SceneEntry quat xyzw (proto3 zero → identity; x002c).
pub fn scene_entry_quat_xyzw(entry: &SceneEntry) -> [f64; 4] {
    let n2 = entry.qx * entry.qx + entry.qy * entry.qy + entry.qz * entry.qz + entry.qw * entry.qw;
    if !n2.is_finite() || n2 < 1e-24 {
        return ENTITY_QUAT_IDENTITY;
    }
    let inv = 1.0 / n2.sqrt();
    [
        entry.qx * inv,
        entry.qy * inv,
        entry.qz * inv,
        entry.qw * inv,
    ]
}

/// SceneEntry scale (x002c).
///
/// - all-zero → unit `(1,1,1)` (proto3 unset)
/// - any non-finite component → unit (poisoned JSON must not seed NaN Frames)
/// - partial zeros / negatives kept (draw may flatten or mirror; host collider ignores scale)
pub fn scene_entry_scale(entry: &SceneEntry) -> [f64; 3] {
    if entry.sx == 0.0 && entry.sy == 0.0 && entry.sz == 0.0 {
        return ENTITY_SCALE_ONE;
    }
    if !entry.sx.is_finite() || !entry.sy.is_finite() || !entry.sz.is_finite() {
        return ENTITY_SCALE_ONE;
    }
    [entry.sx, entry.sy, entry.sz]
}

/// Write identity-or-normalized TRS onto `entry` (for publishers / JSON bridges).
pub fn normalize_scene_entry_trs(entry: &mut SceneEntry) {
    let q = scene_entry_quat_xyzw(entry);
    let s = scene_entry_scale(entry);
    entry.qx = q[0];
    entry.qy = q[1];
    entry.qz = q[2];
    entry.qw = q[3];
    entry.sx = s[0];
    entry.sy = s[1];
    entry.sz = s[2];
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn roundtrip_delta_frame() {
        let frame = RenderFrame {
            tick: 42,
            host_time_ms: 1_000,
            upserts: vec![entity_trs(
                1,
                1.5,
                0.0,
                -2.0,
                [0.0, 0.7071067811865476, 0.0, 0.7071067811865476],
                [2.0, 1.0, 0.5],
                1.0,
                0.2,
                0.1,
                1.0,
            )],
            removes: vec![9],
            host_sig: vec![],
        };
        let bytes = frame.encode_to_vec();
        let decoded = RenderFrame::decode(bytes.as_slice()).expect("decode");
        assert_eq!(decoded.tick, 42);
        assert_eq!(decoded.upserts.len(), 1);
        assert!((decoded.upserts[0].x - 1.5).abs() < f64::EPSILON);
        assert!((decoded.upserts[0].qw - 0.7071067811865476).abs() < 1e-9);
        assert!((decoded.upserts[0].sx - 2.0).abs() < f64::EPSILON);
        assert_eq!(decoded.removes, vec![9]);
    }

    #[test]
    fn roundtrip_scene_manifest() {
        let manifest = SceneManifest {
            room_id: "local".into(),
            revision: 1,
            entries: vec![
                SceneEntry {
                    entity_id: 1,
                    blake3: "fa24848fa7b971718ce6e8dc9bd2d211808fd64e4ddf555feb66d4f82ee1e72a"
                        .into(),
                    x: 0.0,
                    y: 5.0,
                    z: 0.0,
                    half_extent: 0.0,
                    flags: 0,
                    r: 0.2,
                    g: 0.9,
                    b: 0.4,
                    a: 1.0,
                    qx: ENTITY_QUAT_IDENTITY[0],
                    qy: ENTITY_QUAT_IDENTITY[1],
                    qz: ENTITY_QUAT_IDENTITY[2],
                    qw: ENTITY_QUAT_IDENTITY[3],
                    sx: ENTITY_SCALE_ONE[0],
                    sy: ENTITY_SCALE_ONE[1],
                    sz: ENTITY_SCALE_ONE[2],
                },
                SceneEntry {
                    entity_id: 10,
                    blake3: String::new(),
                    x: 0.0,
                    y: 0.25,
                    z: 3.5,
                    half_extent: 0.25,
                    flags: SCENE_FLAG_GRABBABLE,
                    r: 0.9,
                    g: 0.35,
                    b: 0.3,
                    a: 1.0,
                    qx: 0.0,
                    qy: 0.3826834323650898,
                    qz: 0.0,
                    qw: 0.9238795325112867,
                    sx: 1.5,
                    sy: 1.0,
                    sz: 1.0,
                },
            ],
            locomotion: Some(LocomotionConfig {
                mode: LocomotionMode::Noclip.into(),
                move_speed: 4.0,
            }),
            protocol_pin: PROTOCOL_PIN.into(),
            host_pub: vec![1, 2, 3],
            host_attestation: "eyJ.test.attestation".into(),
            world_seal: "eyJ.test.seal".into(),
            host_generation: 1,
        };
        let bytes = manifest.encode_to_vec();
        let decoded = SceneManifest::decode(bytes.as_slice()).expect("decode");
        assert_eq!(decoded.room_id, "local");
        assert_eq!(decoded.revision, 1);
        assert_eq!(decoded.entries.len(), 2);
        assert_eq!(decoded.entries[0].entity_id, 1);
        assert_eq!(
            decoded.entries[0].blake3,
            "fa24848fa7b971718ce6e8dc9bd2d211808fd64e4ddf555feb66d4f82ee1e72a"
        );
        assert!(scene_entry_grabbable(decoded.entries[1].flags));
        assert!((decoded.entries[1].half_extent - 0.25).abs() < f32::EPSILON);
        assert!((decoded.entries[1].qw - 0.9238795325112867).abs() < 1e-9);
        assert!((decoded.entries[1].sx - 1.5).abs() < f64::EPSILON);
        assert_eq!(
            scene_entry_quat_xyzw(&SceneEntry::default()),
            ENTITY_QUAT_IDENTITY
        );
        assert_eq!(scene_entry_scale(&SceneEntry::default()), ENTITY_SCALE_ONE);
        assert_eq!(
            scene_entry_scale(&SceneEntry {
                sx: f64::NAN,
                sy: 1.0,
                sz: 1.0,
                ..Default::default()
            }),
            ENTITY_SCALE_ONE
        );
        assert_eq!(
            scene_entry_scale(&SceneEntry {
                sx: 0.0,
                sy: 2.0,
                sz: 0.0,
                ..Default::default()
            }),
            [0.0, 2.0, 0.0],
            "partial zeros are not identity — only all-zero"
        );
        assert!(!scene_entry_is_box(
            &decoded.entries[0].blake3,
            decoded.entries[0].half_extent,
            decoded.entries[0].entity_id
        ));
        assert!(scene_entry_is_box(
            &decoded.entries[1].blake3,
            decoded.entries[1].half_extent,
            decoded.entries[1].entity_id
        ));
        // blake3 wins over half_extent when both set
        assert!(!scene_entry_is_box("deadbeef", 0.25, 99));
        let loco = decoded.locomotion.expect("locomotion");
        assert_eq!(loco.mode, i32::from(LocomotionMode::Noclip));
        assert!((loco.move_speed - 4.0).abs() < f32::EPSILON);
        assert_eq!(decoded.protocol_pin, PROTOCOL_PIN);
        assert_eq!(decoded.host_pub, vec![1, 2, 3]);
        assert_eq!(decoded.world_seal, "eyJ.test.seal");
        assert_eq!(decoded.host_generation, 1);
        let seal_hex = wire_crypto::manifest_seal_blake3_hex(&decoded);
        assert_eq!(seal_hex.len(), 64);
        let mut cleared = decoded.clone();
        cleared.world_seal.clear();
        assert_eq!(
            wire_crypto::manifest_seal_blake3_hex(&cleared),
            seal_hex,
            "world_seal must not affect seal blake3"
        );
    }

    #[test]
    fn roundtrip_input_event() {
        let event = InputEvent {
            room_token: "dummy.jwt".into(),
            client_time_ms: 99,
            move_x: 0.5,
            move_y: -0.25,
            move_z: 0.0,
            buttons: input_buttons::PRIMARY | input_buttons::JUMP | input_buttons::GRAB,
            grab_entity_id: 14,
            aim_x: 1.0,
            aim_y: 1.5,
            aim_z: 3.0,
            leave: false,
            session_pub: vec![0u8; 32],
            intent_seq: 3,
            intent_sig: vec![0u8; 64],
        };
        let bytes = event.encode_to_vec();
        let decoded = InputEvent::decode(bytes.as_slice()).expect("decode");
        assert_eq!(decoded.room_token, "dummy.jwt");
        assert!((decoded.move_x - 0.5).abs() < f32::EPSILON);
        assert_eq!(
            decoded.buttons,
            input_buttons::PRIMARY | input_buttons::JUMP | input_buttons::GRAB
        );
        assert_eq!(decoded.grab_entity_id, 14);
        assert!((decoded.aim_y - 1.5).abs() < f32::EPSILON);
        assert!(!decoded.leave);
        assert_eq!(decoded.intent_seq, 3);
        assert_eq!(decoded.session_pub.len(), 32);
    }

    #[test]
    fn roundtrip_input_event_leave() {
        let event = InputEvent {
            room_token: "dummy.jwt".into(),
            client_time_ms: 1,
            move_x: 0.0,
            move_y: 0.0,
            move_z: 0.0,
            buttons: 0,
            grab_entity_id: 0,
            aim_x: 0.0,
            aim_y: 0.0,
            aim_z: 0.0,
            leave: true,
            session_pub: vec![],
            intent_seq: 0,
            intent_sig: vec![],
        };
        let decoded = InputEvent::decode(event.encode_to_vec().as_slice()).expect("decode");
        assert!(decoded.leave);
    }

    #[test]
    fn roundtrip_presence_state() {
        let state = PresenceState {
            revision: 2,
            assignments: vec![
                PresenceAssignment {
                    sub: "alice-uuid".into(),
                    entity_id: 3,
                },
                PresenceAssignment {
                    sub: "bob-uuid".into(),
                    entity_id: 4,
                },
            ],
            protocol_pin: PROTOCOL_PIN.into(),
            host_sig: vec![],
            host_generation: 1,
        };
        let bytes = state.encode_to_vec();
        let decoded = PresenceState::decode(bytes.as_slice()).expect("decode");
        assert_eq!(decoded.revision, 2);
        assert_eq!(decoded.assignments.len(), 2);
        assert_eq!(decoded.assignments[0].entity_id, 3);
        assert_eq!(decoded.assignments[1].sub, "bob-uuid");
        assert_eq!(decoded.protocol_pin, PROTOCOL_PIN);
        assert_eq!(decoded.host_generation, 1);
    }

    #[test]
    fn key_helpers() {
        assert_eq!(frame_key("local"), FRAME_KEY_LOCAL);
        assert_eq!(input_key("local"), INPUT_KEY_LOCAL);
        assert_eq!(manifest_key("local"), MANIFEST_KEY_LOCAL);
        assert_eq!(host_control_key("local"), HOST_CONTROL_KEY_LOCAL);
        assert_eq!(presence_key("local"), PRESENCE_KEY_LOCAL);
        assert_eq!(snapshot_key("local"), SNAPSHOT_KEY_LOCAL);
        assert_eq!(presence_key("alpha"), "alchemy/dev/room/alpha/presence");
        assert_ne!(presence_key("alpha"), presence_key("beta"));
        assert_eq!(
            liveliness_key("local", "alice-uuid", "jti-1"),
            format!("{LIVELINESS_KEY_LOCAL_PREFIX}/alice-uuid/jti-1")
        );
        assert_eq!(
            liveliness_wildcard("alpha"),
            "alchemy/dev/room/alpha/alive/**"
        );
        assert_eq!(
            liveliness_binding_from_key("alchemy/dev/room/local/alive/alice-uuid/jti-1"),
            Some(("alice-uuid", "jti-1"))
        );
        assert_eq!(
            liveliness_binding_from_key("alchemy/dev/room/local/alive/alice-uuid"),
            None
        );
        assert_eq!(
            liveliness_binding_from_key("alchemy/dev/room/local/alive/a/b/c"),
            None
        );
        assert!(protocol_pin_matches(PROTOCOL_PIN));
        assert!(!protocol_pin_matches("alchemy-protocol/0.9.0-x0022"));
    }

    /// Shared vectors with world `World.RoomId` (x002e). Keep Elixir tests in sync.
    #[test]
    fn validate_room_id_shared_vectors() {
        let ok = ["local", "alpha", "a", "A_b-9", &"x".repeat(64)];
        for id in ok {
            assert_eq!(validate_room_id(id).unwrap(), id, "accept {id:?}");
        }
        assert_eq!(validate_room_id("").unwrap_err(), RoomIdError::Empty);
        assert_eq!(
            validate_room_id("a/b").unwrap_err(),
            RoomIdError::ContainsSlash
        );
        assert_eq!(
            validate_room_id(&"x".repeat(65)).unwrap_err(),
            RoomIdError::TooLong
        );
        for bad in ["部屋", "a b", "a*b", "a.b", "a@b"] {
            assert_eq!(
                validate_room_id(bad).unwrap_err(),
                RoomIdError::InvalidCharset,
                "reject {bad:?}"
            );
        }
    }

    #[test]
    fn host_control_zenoh_defaults_off() {
        assert!(!host_control_zenoh_enabled_from(None));
        assert!(!host_control_zenoh_enabled_from(Some("")));
        assert!(!host_control_zenoh_enabled_from(Some("0")));
        assert!(!host_control_zenoh_enabled_from(Some("false")));
        assert!(host_control_zenoh_enabled_from(Some("1")));
        assert!(host_control_zenoh_enabled_from(Some("true")));
        assert!(host_control_zenoh_enabled_from(Some("YES")));
    }

    #[test]
    fn decide_frame_tick_consecutive_and_gap() {
        assert_eq!(
            decide_frame_tick(None, 0, FRAME_TICK_GAP_RESYNC),
            FrameTickDecision::Apply
        );
        assert_eq!(
            decide_frame_tick(Some(10), 11, FRAME_TICK_GAP_RESYNC),
            FrameTickDecision::Apply
        );
        assert_eq!(
            decide_frame_tick(Some(10), 12, FRAME_TICK_GAP_RESYNC),
            FrameTickDecision::ResyncSnapshot
        );
        assert_eq!(
            decide_frame_tick(Some(10), 10, FRAME_TICK_GAP_RESYNC),
            FrameTickDecision::DropStale
        );
        assert_eq!(
            decide_frame_tick(Some(10), 9, FRAME_TICK_GAP_RESYNC),
            FrameTickDecision::DropStale
        );
        // Wider tolerance for tests / future tuning.
        assert_eq!(decide_frame_tick(Some(10), 12, 2), FrameTickDecision::Apply);
        assert_eq!(
            decide_frame_tick(Some(10), 13, 2),
            FrameTickDecision::ResyncSnapshot
        );
    }
}
