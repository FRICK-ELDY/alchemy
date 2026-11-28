//! Alchemy wire contracts (Protobuf).
//!
//! # Guarantees
//! - `PROTOCOL_PIN` identifies the schema generation expected by peers (also on Manifest/Presence).
//! - `RenderFrame` (delta upserts/removes + host_sig), `SceneManifest` (+ `LocomotionConfig`, host_pub),
//!   `InputEvent` (session Ed25519), `PresenceState` (`sub`→entity) encode/decode.
//! - [`wire_crypto`] sign/verify helpers for Intent / Frame (x0023).
//! - [`wire_aead`] Room-key XChaCha20Poly1305 for Zenoh payloads (x0069).
//! - [`port_framing`] Erlang `{:packet, 4}` helpers for `blake3-hash` / `manifest-seal-hash --serve` (x0075).
//! - [`manifest_seal_json`] JSON → SceneManifest for `manifest-seal-hash` (x0039 / x0075).
//! - [`demo_scene_vector`] DemoScene ManifestSeal blake3 **test vector** only (x0079).
//! - [`vcs_object`] Spatial VCS Tree/Commit deterministic CBOR codec + id (x00c8; **does not bump** pin —
//!   content-addressed objects, not peer wire).
//! - [`vcs_object_port`] JSON Port contract for `vcs-object` bin (TRS = f32 LE b64 only).
//! - [`sim_capability`] role → Intent/HTTP capability bits (x002a; not a wire schema).
//! - [`item_ref`] CAS spawn pointer draft (`blake3` only; x0055; **does not bump** pin —
//!   not on Manifest / Frame / Intent yet).
//! - [`generation_gate`] monotonic `(host_generation, revision)` (x003a).
//! - [`decode_limits`] class-specific pre/post decode ceilings (x003b; **does not bump** pin —
//!   local validation, wire-compatible with x003a peers).
//! - Delivery class table: [`peer-delivery.md`](../peer-delivery.md). Frame tick gap → snapshot
//!   ([`decide_frame_tick`], [`FRAME_TICK_GAP_RESYNC`]; x0024). Spatial Interest tick gates:
//!   Room [`decide_room_frame_tick`] (x00c0; default client path), cell [`decide_cell_frame_tick`]
//!   (x00c3; opt-in cell mainline · x00c4 — Room `…/frame` not subscribed when enabled).
//!
//! # Non-guarantees
//! - Bit-packing / prediction coding, DrawCommand taxonomy, audio/tracking frames.
//! - Cryptographic verification of `InputEvent.room_token` (host-server JWKS).
//! - Rapier / Dual-AST Kernel execution (see `alchemy-shared-sim` EntityKind index; x0030).
//! - Frame scale animation / non-uniform collider resize (x0029 emits unit scale).
//! - Caps on Presence / JWT claim; invite ACL; reliable-delivery ACK / event-id channel.
//! - Manifest scale resizing Rapier colliders (half_extent stays definition size).
//! - Locomotion module list UI / suppress sources.
//! - Tenant namespace `alchemy/tenant/...` (dev keys: `alchemy/dev/room/{room_id}/{wire_cap}/…` · x004e).
//! - Room capacity beyond [`USER_ROOT_SLOTS_MAX`] (DoS ceiling) / non-contiguous user_root ids (x004d / x009b).
//! - WAN liveliness ACL beyond Relay Agent MVP (x004e).
//! - Full host handoff UX / Wake-on-Join (generation advance is the contract only).
//! - Router max-payload / per-peer QoS (app-level decode limits only; x003b).
//! - Long-running fuzz campaigns (x004c ships short CI smoke + seed corpus only).
//! - `wire_cap` topic-name secrecy (x0069: confidentiality is Room-key AEAD).
//! - ItemRef `repo@rev` / spawn execution / wire carriage (x0055 draft only; → x0063+).
//! - Spatial VCS push / DB / assets Port pool (x00c8 ships codec + `vcs-object` bin only).

#![forbid(unsafe_code)]

pub use prost::Message;

pub mod control_plane;
pub mod decode_limits;
pub mod demo_scene_vector;
pub mod edit_field;
pub mod generation_gate;
pub mod host_ipc;
pub mod http_pop;
pub mod item_ref;
pub mod manifest_seal_json;
pub mod port_framing;
pub mod sim_capability;
pub mod vcs_object;
pub mod vcs_object_port;
pub mod wire_aead;
pub mod wire_crypto;

pub use edit_field::{
    blake3_hash_32, decode_color_rgba, decode_f64, decode_flags, encode_color_rgba, encode_f64,
    encode_flags, field_value_hash, hash_blake3_hex, hash_color_rgba, hash_f64, hash_flags,
    is_pos_field_id, known_field_id, pos_field_axis,
};

/// Bump when `proto/` has an incompatible wire change.
///
/// x0069: Zenoh payloads are Room-key AEAD envelopes (incompatible with x004e plain protobuf peers).
/// x0081: inventory control JSON drops `room_token` (session sig); host HTTP adds PoP headers.
/// x009b: user_root entity band moves to high ids (`USER_ROOT_ENTITY_FIRST`); peers must agree.
/// x00af: `RenderFrame.sim_tick` / `redundancy` / `acked_intent_seq`; `Entity.holder`;
///        Intent `release_v*` / `release_w*` (throw apply still host-local).
/// x00b2: `EditProposal` / `EditCommit` on `…/edit` (Spawn/Despawn ops).
/// x00b3: Reparent/SetField + `RenderFrame.scene_rev`; Commit tip = host live_scene_rev.
/// x00b7: `EditSave` overlay→canonical; Commit tip unchanged on Save.
/// x00b8: `SceneEntry.parent_entity_id`; Save folds live parent/SetField.
/// x00f2 POS_X/Y/Z is additive on `EditSetField.field_id` (no pin bump; unknown → OP_UNSUPPORTED).
pub const PROTOCOL_PIN: &str = "alchemy-protocol/0.24.0-x00b8";

/// `EditCommit.reject_reason` codes (x00b2 / x00b3 / x00b7).
pub mod edit_reject {
    pub const NONE: u32 = 0;
    pub const AUTH: u32 = 1;
    pub const RATE: u32 = 2;
    pub const SIG: u32 = 3;
    pub const OP_UNSUPPORTED: u32 = 4;
    pub const OPS_COUNT: u32 = 5;
    pub const PAYLOAD: u32 = 6;
    pub const WORLD: u32 = 7;
    /// Optimistic lock failed (`expected_parent` / `expected_value_hash`).
    pub const STALE_PRECONDITION: u32 = 8;
    pub const NOT_FOUND: u32 = 9;
    pub const CYCLE: u32 = 10;
    /// Ready delivery / FrameBridge queue / tick-barrier apply failed
    /// (mirrors rolled back when applicable; tip unchanged).
    pub const APPLY: u32 = 11;
    /// Scene writer forbidden / active-host Save rejected by world (x00b7).
    pub const SAVE_FORBIDDEN: u32 = 12;
    /// Save packed CAS exhausted / revision conflict (x00b7).
    pub const SAVE_CONFLICT: u32 = 13;
    /// SoftLease held by another actor (x00e2 structural edit lock).
    pub const SOFT_LEASE: u32 = 14;
}

/// `EditSetField.field_id` (x00b3 / x00f2). Value encodings are little-endian / UTF-8 as noted.
pub mod edit_field_id {
    /// `u32` LE flags (`SceneEntry.flags` / live mirror).
    pub const FLAGS: u32 = 1;
    /// 4×`f32` LE = RGBA.
    pub const COLOR_RGBA: u32 = 2;
    /// 64-char lowercase hex blake3 (UTF-8), same as `SceneEntry.blake3`.
    pub const BLAKE3: u32 = 3;
    /// World position X (`f64` LE) — Inspector scrub Commit (x00f2).
    pub const POS_X: u32 = 4;
    /// World position Y (`f64` LE).
    pub const POS_Y: u32 = 5;
    /// World position Z (`f64` LE).
    pub const POS_Z: u32 = 6;
}

/// Wire `Entity.holder`: nobody holds the entity.
pub const ENTITY_HOLDER_NONE: u32 = 0;

/// Default linear release-speed clamp (m/s) for Intent `release_v*` (x00af).
pub const RELEASE_LINEAR_SPEED_MAX: f32 = 20.0;

/// Default angular release-speed clamp (rad/s) for Intent `release_w*` (x00af).
pub const RELEASE_ANGULAR_SPEED_MAX: f32 = 40.0;

/// Identity orientation on the wire (`Entity.qx..qw`, xyzw).
pub const ENTITY_QUAT_IDENTITY: [f64; 4] = [0.0, 0.0, 0.0, 1.0];

/// Unit scale on the wire (`Entity.sx..sz`).
pub const ENTITY_SCALE_ONE: [f64; 3] = [1.0, 1.0, 1.0];

/// `SceneEntry.flags`: entity may be Grab-held (x0027).
pub const SCENE_FLAG_GRABBABLE: u32 = 1;

/// Default free-move speed (m/s) when Manifest `move_speed` is 0.
/// Shared by host Noclip and client Settings display (x000a).
pub const DEFAULT_MOVE_SPEED: f32 = 4.0;

/// First entity id reserved for Presence user_roots (protocol band rule; x004d / x009b).
///
/// High range so Manifest-driven width (5+) cannot collide with low demo content ids
/// (coin=`1`, boxes=`10..`).
pub const USER_ROOT_ENTITY_FIRST: u32 = 0x1000_0000;

/// Implementation / DoS ceiling on concurrent user_roots (id band width).
///
/// Room Presence capacity is **not** this constant — it comes from contiguous
/// Manifest `user_root` entries via [`user_root_slots_from_entries`] (x004d / x009b).
/// Raising room size within this ceiling does **not** require another pin bump.
pub const USER_ROOT_SLOTS_MAX: usize = 32;

/// Inclusive last entity id of the user_root band ([`USER_ROOT_ENTITY_FIRST`] + max − 1).
pub const fn user_root_entity_last() -> u32 {
    USER_ROOT_ENTITY_FIRST + USER_ROOT_SLOTS_MAX as u32 - 1
}

/// Why [`user_root_slots_from_entries`] rejected Manifest entries.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UserRootSlotsError {
    Empty,
    Duplicate { entity_id: u32 },
    Gap { missing_entity_id: u32 },
}

impl std::fmt::Display for UserRootSlotsError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Empty => write!(f, "Manifest has no user_root entries"),
            Self::Duplicate { entity_id } => {
                write!(f, "duplicate user_root entity_id={entity_id}")
            }
            Self::Gap { missing_entity_id } => {
                write!(
                    f,
                    "user_root ids must be contiguous from {}; missing {missing_entity_id}",
                    USER_ROOT_ENTITY_FIRST
                )
            }
        }
    }
}

impl std::error::Error for UserRootSlotsError {}

/// True when `flags` includes [`SCENE_FLAG_GRABBABLE`].
pub const fn scene_entry_grabbable(flags: u32) -> bool {
    flags & SCENE_FLAG_GRABBABLE != 0
}

/// True when `entity_id` is in the Presence user_root **band** (protocol ceiling).
///
/// Room capacity may be smaller; see [`user_root_slots_from_entries`].
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

/// Wire `Entity.holder` from a user_root slot index (`0..USER_ROOT_SLOTS_MAX`).
///
/// Returns [`ENTITY_HOLDER_NONE`] when `slot` is out of range (callers should not
/// pass page `NO_HOLDER=255` here — map that to none first).
pub const fn entity_holder_from_slot(slot: u8) -> u32 {
    if (slot as usize) >= USER_ROOT_SLOTS_MAX {
        ENTITY_HOLDER_NONE
    } else {
        slot as u32 + 1
    }
}

/// Inverse of [`entity_holder_from_slot`]. `None` when holder is none / invalid.
pub const fn entity_holder_slot(holder: u32) -> Option<u8> {
    if holder == ENTITY_HOLDER_NONE || holder > USER_ROOT_SLOTS_MAX as u32 {
        None
    } else {
        Some((holder - 1) as u8)
    }
}

/// Clamp / sanitize Intent release linear+angular velocity (x00af).
///
/// Non-finite components become 0. Magnitude above `lin_max` / `ang_max` is scaled down.
/// Does **not** apply impulse to sim — host callers decide when to use the result.
pub fn clamp_release_velocity(
    lin: [f32; 3],
    ang: [f32; 3],
    lin_max: f32,
    ang_max: f32,
) -> ([f32; 3], [f32; 3]) {
    (
        clamp_vec3_speed(lin, lin_max),
        clamp_vec3_speed(ang, ang_max),
    )
}

fn clamp_vec3_speed(v: [f32; 3], max_speed: f32) -> [f32; 3] {
    let [x, y, z] = v;
    if !x.is_finite() || !y.is_finite() || !z.is_finite() {
        return [0.0, 0.0, 0.0];
    }
    let max_speed = if max_speed.is_finite() && max_speed > 0.0 {
        max_speed
    } else {
        return [0.0, 0.0, 0.0];
    };
    let speed_sq = x * x + y * y + z * z;
    if speed_sq <= max_speed * max_speed {
        return [x, y, z];
    }
    let speed = speed_sq.sqrt();
    if !speed.is_finite() || speed <= 0.0 {
        return [0.0, 0.0, 0.0];
    }
    let s = max_speed / speed;
    [x * s, y * s, z * s]
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

/// Build an [`Entity`] with explicit TRS (x0029). `holder` defaults to none.
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
        holder: ENTITY_HOLDER_NONE,
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

/// Fixture `wire_cap` for unit tests / `ROOM_ID=local` key constants (x004e).
///
/// Real rooms get a random cap from world-server; this value is **not** a WAN secret.
pub const LOCAL_WIRE_CAP: &str = "localdevwirecap00000001";

/// Fixture Room AEAD key for debug Token bypass / unit tests only (x0069). **Not for WAN.**
pub const LOCAL_ROOM_KEY: [u8; wire_aead::ROOM_KEY_LEN] = *b"alchemy-local-room-key-x0069!!!!";

/// Default Zenoh key for local room frames (high-frequency pose delta).
pub const FRAME_KEY_LOCAL: &str = "alchemy/dev/room/local/localdevwirecap00000001/frame";

/// Local origin cell frame key `…/frame/cell/0_0_0` (x00be; x00c0 publishes per dirty cell).
pub const FRAME_CELL_KEY_LOCAL_ORIGIN: &str =
    "alchemy/dev/room/local/localdevwirecap00000001/frame/cell/0_0_0";

/// Fine→LodCell stride for Avatar sync LOD (x00c7). `LodCell ≈ CELL_SIZE_M * stride` metres.
pub const LOD_CELL_STRIDE: i32 = 3;

/// Local origin lod frame key `…/frame/lod/0_0_0` (x00c7 mid-band sync LOD).
pub const FRAME_LOD_KEY_LOCAL_ORIGIN: &str =
    "alchemy/dev/room/local/localdevwirecap00000001/frame/lod/0_0_0";

/// Default Zenoh key for local room inputs.
pub const INPUT_KEY_LOCAL: &str = "alchemy/dev/room/local/localdevwirecap00000001/input";

/// Default Zenoh key for local room scene manifest (low-frequency blob refs).
pub const MANIFEST_KEY_LOCAL: &str = "alchemy/dev/room/local/localdevwirecap00000001/manifest";

/// Default Zenoh key for local host control (Slice E stub: shutdown).
pub const HOST_CONTROL_KEY_LOCAL: &str =
    "alchemy/dev/room/local/localdevwirecap00000001/host_control";

/// Default Zenoh key for local inventory spawn (x006d).
pub const INVENTORY_SPAWN_KEY_LOCAL: &str =
    "alchemy/dev/room/local/localdevwirecap00000001/inventory/spawn";

/// Default Zenoh key for local inventory despawn (x007b).
pub const INVENTORY_DESPAWN_KEY_LOCAL: &str =
    "alchemy/dev/room/local/localdevwirecap00000001/inventory/despawn";

/// Default Zenoh key for local room presence (low-frequency sub→entity; x0015).
pub const PRESENCE_KEY_LOCAL: &str = "alchemy/dev/room/local/localdevwirecap00000001/presence";

/// Default Zenoh key for local room snapshot queryable (x0023).
pub const SNAPSHOT_KEY_LOCAL: &str = "alchemy/dev/room/local/localdevwirecap00000001/snapshot";

/// Default Zenoh key for local planned-handover sim_state queryable (x00b0).
pub const SIM_STATE_KEY_LOCAL: &str = "alchemy/dev/room/local/localdevwirecap00000001/sim_state";

/// Default Zenoh key for local edit lane (x00b2; proposal + commit share the key).
pub const EDIT_KEY_LOCAL: &str = "alchemy/dev/room/local/localdevwirecap00000001/edit";

/// Payload accepted by host-server on `host_control_key(room_id)` (Slice E stub).
pub const HOST_CONTROL_SHUTDOWN: &[u8] = b"shutdown";

/// Env var that opts into unauthenticated Zenoh `host_control` (x001e; default off).
pub const HOST_CONTROL_ZENOH_ENV: &str = "ALCHEMY_HOST_CONTROL";

/// Deprecated env name from x00bf / x00c4 cell-frame opt-in (`ALCHEMY_CELL_FRAME`).
///
/// **x00c5:** setting this variable does **not** change frame subscribe mode. Mode is
/// selected only via [`ROOM_FRAME_SUB_ENV`] (`ALCHEMY_ROOM_FRAME`). Clients may log a
/// deprecation warning when this env is present. Prefer unset (cell mainline) or
/// [`ROOM_FRAME_SUB_ENV`] for Room-only opt-out.
#[deprecated(
    since = "x00c5",
    note = "ignored for frame mode; use ALCHEMY_ROOM_FRAME for Room-only opt-out"
)]
pub const CELL_FRAME_SUB_ENV: &str = "ALCHEMY_CELL_FRAME";

/// Env var that opts into Room `…/frame` as the exclusive frame path
/// (x00c5 Spatial Interest; default **off** — Moore cell mainline).
///
/// When enabled, clients subscribe to Room `…/frame` only (cell keys are **not** subscribed).
/// When unset/false, clients use Moore cell mainline; gap→resync still uses Room `…/snapshot`.
pub const ROOM_FRAME_SUB_ENV: &str = "ALCHEMY_ROOM_FRAME";

/// Moore neighborhood size including the center cell (`3³`).
pub const MOORE_NEIGHBORHOOD_LEN: usize = 27;

/// Shared truthy parse for Alchemy opt-in env flags (`1` / `true` / `yes`, case variants).
fn env_flag_enabled_from(value: Option<&str>) -> bool {
    matches!(
        value,
        Some("1") | Some("true") | Some("TRUE") | Some("yes") | Some("YES")
    )
}

/// Parse whether Zenoh host_control is enabled from an env value (testable).
///
/// Accepted truthy: `1`, `true`, `TRUE`, `yes`, `YES`. Anything else (incl. unset) is off.
pub fn host_control_zenoh_enabled_from(value: Option<&str>) -> bool {
    env_flag_enabled_from(value)
}

/// Read [`HOST_CONTROL_ZENOH_ENV`]; default **false** so WAN peers cannot stop host.
pub fn host_control_zenoh_enabled() -> bool {
    host_control_zenoh_enabled_from(std::env::var(HOST_CONTROL_ZENOH_ENV).ok().as_deref())
}

/// Parse legacy `ALCHEMY_CELL_FRAME` (testable). **x00c5:** not used for mode selection.
///
/// Same truthy set as [`host_control_zenoh_enabled_from`]. Historical default off.
#[deprecated(
    since = "x00c5",
    note = "ignored for frame mode; use room_frame_sub_enabled_from / ALCHEMY_ROOM_FRAME"
)]
pub fn cell_frame_sub_enabled_from(value: Option<&str>) -> bool {
    env_flag_enabled_from(value)
}

/// Read `ALCHEMY_CELL_FRAME`; legacy x00bf/x00c4 API. Prefer [`room_frame_sub_enabled`].
#[deprecated(
    since = "x00c5",
    note = "ignored for frame mode; use room_frame_sub_enabled / ALCHEMY_ROOM_FRAME"
)]
pub fn cell_frame_sub_enabled() -> bool {
    // Avoid deprecated const / self-call noise; string is the public env name.
    #[allow(deprecated)]
    {
        cell_frame_sub_enabled_from(std::env::var("ALCHEMY_CELL_FRAME").ok().as_deref())
    }
}

/// Parse whether Room-frame opt-out is enabled (testable; x00c5).
///
/// Same truthy set as [`host_control_zenoh_enabled_from`]. Default off → cell mainline.
pub fn room_frame_sub_enabled_from(value: Option<&str>) -> bool {
    env_flag_enabled_from(value)
}

/// Read [`ROOM_FRAME_SUB_ENV`]; default **false** so clients use Moore cell mainline.
/// When true, Room `…/frame` is the exclusive tick path (cell not subscribed; x00c5).
pub fn room_frame_sub_enabled() -> bool {
    room_frame_sub_enabled_from(std::env::var(ROOM_FRAME_SUB_ENV).ok().as_deref())
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

/// Why [`validate_wire_cap`] rejected a value.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WireCapError {
    Empty,
    ContainsSlash,
    TooLong,
    InvalidCharset,
}

impl std::fmt::Display for WireCapError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Empty => write!(f, "wire_cap must not be empty"),
            Self::ContainsSlash => write!(f, "wire_cap must not contain '/'"),
            Self::TooLong => write!(f, "wire_cap must be at most 64 bytes"),
            Self::InvalidCharset => {
                write!(f, "wire_cap must match [A-Za-z0-9_-]+")
            }
        }
    }
}

impl std::error::Error for WireCapError {}

/// Max byte length for [`validate_wire_cap`] (x004e; Zenoh key segment).
pub const WIRE_CAP_MAX_BYTES: usize = 64;

/// Validate `wire_cap` for Zenoh key segments (same charset as [`validate_room_id`]).
pub fn validate_wire_cap(wire_cap: &str) -> Result<&str, WireCapError> {
    if wire_cap.is_empty() {
        return Err(WireCapError::Empty);
    }
    if wire_cap.contains('/') {
        return Err(WireCapError::ContainsSlash);
    }
    if wire_cap.len() > WIRE_CAP_MAX_BYTES {
        return Err(WireCapError::TooLong);
    }
    if !wire_cap
        .bytes()
        .all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'-')
    {
        return Err(WireCapError::InvalidCharset);
    }
    Ok(wire_cap)
}

fn room_prefix(room_id: &str, wire_cap: &str) -> String {
    format!("alchemy/dev/room/{room_id}/{wire_cap}")
}

/// Build a frame key (caller should [`validate_room_id`] + [`validate_wire_cap`]).
pub fn frame_key(room_id: &str, wire_cap: &str) -> String {
    format!("{}/frame", room_prefix(room_id, wire_cap))
}

/// Why [`parse_cell_id_segment`] rejected a value (x00be).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CellIdSegmentError {
    Empty,
    ContainsSlash,
    WrongPartCount,
    InvalidInteger,
}

impl std::fmt::Display for CellIdSegmentError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Empty => write!(f, "cell id segment must not be empty"),
            Self::ContainsSlash => write!(f, "cell id segment must not contain '/'"),
            Self::WrongPartCount => write!(f, "cell id segment must be cx_cy_cz"),
            Self::InvalidInteger => write!(f, "cell id axis must be a signed decimal integer"),
        }
    }
}

impl std::error::Error for CellIdSegmentError {}

/// Format a cell id as one Zenoh key segment (`cx_cy_cz`, e.g. `0_0_0`, `-1_-2_0`).
///
/// Same integer axes as shared-sim `CellId` (`CELL_SIZE_M=64` center origin); this crate
/// only formats indices and does not convert world coordinates.
pub fn format_cell_id_segment(cx: i32, cy: i32, cz: i32) -> String {
    format!("{cx}_{cy}_{cz}")
}

fn parse_i32_prefix(s: &str) -> Result<(i32, &str), CellIdSegmentError> {
    let bytes = s.as_bytes();
    if bytes.is_empty() {
        return Err(CellIdSegmentError::InvalidInteger);
    }
    let mut i = 0;
    if bytes[0] == b'-' {
        i = 1;
        if i >= bytes.len() {
            return Err(CellIdSegmentError::InvalidInteger);
        }
    }
    let digit_start = i;
    while i < bytes.len() && bytes[i].is_ascii_digit() {
        i += 1;
    }
    if i == digit_start {
        return Err(CellIdSegmentError::InvalidInteger);
    }
    let n = s[..i]
        .parse::<i32>()
        .map_err(|_| CellIdSegmentError::InvalidInteger)?;
    Ok((n, &s[i..]))
}

/// Parse a cell id segment produced by [`format_cell_id_segment`].
///
/// Guarantees: exactly three signed decimal axes separated by `_` (supports negatives
/// such as `-1_-2_0`). Rejects `/`, empty parts, and trailing junk.
pub fn parse_cell_id_segment(s: &str) -> Result<[i32; 3], CellIdSegmentError> {
    if s.is_empty() {
        return Err(CellIdSegmentError::Empty);
    }
    if s.contains('/') {
        return Err(CellIdSegmentError::ContainsSlash);
    }
    let mut vals = [0i32; 3];
    let mut rest = s;
    for (i, slot) in vals.iter_mut().enumerate() {
        let (n, next) = parse_i32_prefix(rest)?;
        *slot = n;
        if i < 2 {
            rest = next
                .strip_prefix('_')
                .ok_or(CellIdSegmentError::WrongPartCount)?;
        } else if !next.is_empty() {
            return Err(CellIdSegmentError::WrongPartCount);
        }
    }
    Ok(vals)
}

/// Build a cell-scoped frame key (x00be Spatial Interest · direction A).
///
/// Room [`frame_key`] remains the tick authority / compatibility path. Caller should
/// [`validate_room_id`] + [`validate_wire_cap`].
pub fn frame_cell_key(room_id: &str, wire_cap: &str, cx: i32, cy: i32, cz: i32) -> String {
    format!(
        "{}/frame/cell/{}",
        room_prefix(room_id, wire_cap),
        format_cell_id_segment(cx, cy, cz)
    )
}

/// Moore neighborhood of `cx,cy,cz` including the center (`3×3×3` = [`MOORE_NEIGHBORHOOD_LEN`]).
///
/// **x00c6 client mainline** declares one subscriber per cell (≤27), not a `|` union.
/// [`frame_cell_interest_keyexpr`] remains for tests / optional union experiments. Does not
/// convert world coordinates (indices only). Axis math uses `saturating_add`; near `i32`
/// extremes neighbors can collide — callers that build a `|` keyexpr should go through
/// [`frame_cell_interest_keyexpr`] (dedups).
pub fn moore_neighborhood_cells(cx: i32, cy: i32, cz: i32) -> [[i32; 3]; MOORE_NEIGHBORHOOD_LEN] {
    let mut out = [[0i32; 3]; MOORE_NEIGHBORHOOD_LEN];
    let mut i = 0;
    for dx in -1i32..=1 {
        for dy in -1i32..=1 {
            for dz in -1i32..=1 {
                out[i] = [
                    cx.saturating_add(dx),
                    cy.saturating_add(dy),
                    cz.saturating_add(dz),
                ];
                i += 1;
            }
        }
    }
    debug_assert_eq!(i, MOORE_NEIGHBORHOOD_LEN);
    out
}

/// Zenoh keyexpr union of [`frame_cell_key`] for each cell (`a|b|c…`).
///
/// Empty `cells` yields an empty string (caller must not declare a subscriber on it).
/// Duplicate cell indices are emitted once (stable first-seen order).
///
/// **Not client mainline** (x00c6 uses per-key ≤27). Observed on local peer (Zenoh 1.10.1,
/// `tcp/127.0.0.1:7447`): long Moore unions (~27 keys) **declare OK** but delivered **zero**
/// samples — keep as test / optional optimization only.
pub fn frame_cell_interest_keyexpr(room_id: &str, wire_cap: &str, cells: &[[i32; 3]]) -> String {
    let mut out = String::new();
    let mut seen: Vec<[i32; 3]> = Vec::with_capacity(cells.len());
    for &[cx, cy, cz] in cells {
        let cell = [cx, cy, cz];
        if seen.contains(&cell) {
            continue;
        }
        seen.push(cell);
        if !out.is_empty() {
            out.push('|');
        }
        out.push_str(&frame_cell_key(room_id, wire_cap, cx, cy, cz));
    }
    out
}

/// ACL-aligned `…/frame/cell/*` keyexpr (WAN ACL shape).
///
/// **Not client default mainline** (x00c6: Moore per-key). Kept for ACL docs, tests, and
/// debug/計測. Receiving all dirty cells unbounded is incompatible with the VR Interest budget.
pub fn frame_cell_wildcard_key(room_id: &str, wire_cap: &str) -> String {
    format!("{}/frame/cell/*", room_prefix(room_id, wire_cap))
}

/// Map a fine `CellId` (`CELL_SIZE_M=64`) to a coarse LodCell (x00c7).
///
/// Uses Euclidean (floor) division so negatives stay contiguous:
/// `cell_to_lod_cell(-1, 0, 0) == [-1, 0, 0]` with [`LOD_CELL_STRIDE`] = 3.
pub fn cell_to_lod_cell(cx: i32, cy: i32, cz: i32) -> [i32; 3] {
    [
        cx.div_euclid(LOD_CELL_STRIDE),
        cy.div_euclid(LOD_CELL_STRIDE),
        cz.div_euclid(LOD_CELL_STRIDE),
    ]
}

/// Build a lod-scoped frame key (x00c7 Avatar sync LOD · mid band).
///
/// Separate from [`frame_cell_key`] so near-band full-rate dirty is not thinned on a shared
/// key. Caller should [`validate_room_id`] + [`validate_wire_cap`].
pub fn frame_lod_key(room_id: &str, wire_cap: &str, lx: i32, ly: i32, lz: i32) -> String {
    format!(
        "{}/frame/lod/{}",
        room_prefix(room_id, wire_cap),
        format_cell_id_segment(lx, ly, lz)
    )
}

/// [`frame_lod_key`] for the LodCell covering fine cell `(cx,cy,cz)`.
pub fn frame_lod_key_from_cell(room_id: &str, wire_cap: &str, cx: i32, cy: i32, cz: i32) -> String {
    let [lx, ly, lz] = cell_to_lod_cell(cx, cy, cz);
    frame_lod_key(room_id, wire_cap, lx, ly, lz)
}

/// ACL-aligned `…/frame/lod/*` keyexpr (WAN ACL shape · x00c7).
///
/// Wildcard is for ACL / debug only — client mid-band mainline is LodCell Moore per-key ≤27.
pub fn frame_lod_wildcard_key(room_id: &str, wire_cap: &str) -> String {
    format!("{}/frame/lod/*", room_prefix(room_id, wire_cap))
}

/// Build an input key (caller should [`validate_room_id`] + [`validate_wire_cap`]).
pub fn input_key(room_id: &str, wire_cap: &str) -> String {
    format!("{}/input", room_prefix(room_id, wire_cap))
}

/// Build a manifest key (caller should [`validate_room_id`] + [`validate_wire_cap`]).
pub fn manifest_key(room_id: &str, wire_cap: &str) -> String {
    format!("{}/manifest", room_prefix(room_id, wire_cap))
}

/// Build a host-control key (caller should [`validate_room_id`] + [`validate_wire_cap`]).
pub fn host_control_key(room_id: &str, wire_cap: &str) -> String {
    format!("{}/host_control", room_prefix(room_id, wire_cap))
}

/// Low-frequency inventory spawn request (x006d). Not Intent; host verifies Room Token.
pub fn inventory_spawn_key(room_id: &str, wire_cap: &str) -> String {
    format!("{}/inventory/spawn", room_prefix(room_id, wire_cap))
}

/// Low-frequency inventory despawn request (x007b). Not Intent; host verifies session sig (x0081).
pub fn inventory_despawn_key(room_id: &str, wire_cap: &str) -> String {
    format!("{}/inventory/despawn", room_prefix(room_id, wire_cap))
}

/// Build a presence key (caller should [`validate_room_id`] + [`validate_wire_cap`]).
pub fn presence_key(room_id: &str, wire_cap: &str) -> String {
    format!("{}/presence", room_prefix(room_id, wire_cap))
}

/// Build a snapshot queryable key (x0023 / x004e).
pub fn snapshot_key(room_id: &str, wire_cap: &str) -> String {
    format!("{}/snapshot", room_prefix(room_id, wire_cap))
}

/// Build a planned-handover sim_state queryable key (x00b0 / mvcc_xpbd §4.8).
pub fn sim_state_key(room_id: &str, wire_cap: &str) -> String {
    format!("{}/sim_state", room_prefix(room_id, wire_cap))
}

/// Build the deterministic edit-lane key (x00b2 / mvcc_xpbd §4.7).
///
/// Clients put [`EditProposal`]; host puts [`EditCommit`]. Distinct [`wire_aead::AeadKind`]s
/// separate the envelopes on the same topic.
pub fn edit_key(room_id: &str, wire_cap: &str) -> String {
    format!("{}/edit", room_prefix(room_id, wire_cap))
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
///
/// Strict monotonic gate (legacy / non-interest callers). Spatial Interest cell frames
/// use [`decide_cell_frame_tick`]; Room frames use [`decide_room_frame_tick`].
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

/// Room `…/frame` tick gate (x00c0; Room opt-out path after x00c5).
///
/// Introduced for Spatial Interest when Room could override a same-tick partial cell.
/// Runtime 併読 is gone under cell mainline (x00c4/x00c5); this gate still applies when
/// `ALCHEMY_ROOM_FRAME=1` selects Room-only subscribe, and in unit tests.
///
/// Same as [`decide_frame_tick`], except `tick == prev` **Applies** when the previous
/// commit was a partial cell (`last_from_room == false`), so Room full dirty + removes
/// can override. When `last_from_room` is already true, same tick stays [`DropStale`]
/// (duplicate Room / snapshot-stamped tick regression guard · x0024).
pub fn decide_room_frame_tick(
    last: Option<u64>,
    tick: u64,
    max_gap: u64,
    last_from_room: bool,
) -> FrameTickDecision {
    decide_partial_frame_tick(last, tick, max_gap, last_from_room)
}

/// Cell interest tick gate under Spatial Interest (x00c3).
///
/// Same decision shape as [`decide_room_frame_tick`]: `tick == prev` **Applies** when the
/// previous commit was also a partial cell (`last_from_room == false`), so multiple cell
/// samples for one `net_tick` can merge. After a Room/snapshot commit (`last_from_room`),
/// same-tick cell stays [`DropStale`].
pub fn decide_cell_frame_tick(
    last: Option<u64>,
    tick: u64,
    max_gap: u64,
    last_from_room: bool,
) -> FrameTickDecision {
    decide_partial_frame_tick(last, tick, max_gap, last_from_room)
}

/// Shared gate for Room override and multi-cell same-tick merge (x00c0 / x00c3).
fn decide_partial_frame_tick(
    last: Option<u64>,
    tick: u64,
    max_gap: u64,
    last_from_room: bool,
) -> FrameTickDecision {
    let Some(prev) = last else {
        return FrameTickDecision::Apply;
    };
    if tick < prev {
        return FrameTickDecision::DropStale;
    }
    if tick == prev {
        return if last_from_room {
            FrameTickDecision::DropStale
        } else {
            FrameTickDecision::Apply
        };
    }
    let advance = tick - prev;
    if advance <= max_gap {
        FrameTickDecision::Apply
    } else {
        FrameTickDecision::ResyncSnapshot
    }
}

/// Default Zenoh liveliness key prefix for local room (x0019 / x004e; `sub` appended).
pub const LIVELINESS_KEY_LOCAL_PREFIX: &str =
    "alchemy/dev/room/local/localdevwirecap00000001/alive";

/// Build a liveliness token key bound to Room Token `jti` (x0022 / x004e).
///
/// `sub` / `jti` must not contain `/`. Host reclaims on Delete only when `jti` matches
/// the last verified Input for that `sub`.
pub fn liveliness_key(room_id: &str, wire_cap: &str, sub: &str, jti: &str) -> String {
    format!("{}/alive/{sub}/{jti}", room_prefix(room_id, wire_cap))
}

/// Host wildcard subscribe key for room liveliness (x0019 / x0022 / x004e).
pub fn liveliness_wildcard(room_id: &str, wire_cap: &str) -> String {
    format!("{}/alive/**", room_prefix(room_id, wire_cap))
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
include!(concat!(env!("OUT_DIR"), "/alchemy.edit.rs"));

/// Room Presence / user_root capacity from Manifest entries (x004d).
///
/// Guarantees: counts ids in the protocol band; requires contiguous
/// `FIRST..FIRST+n-1` with `1 <= n <= USER_ROOT_SLOTS_MAX`; rejects duplicates.
/// Non-guarantees: spawn pose validity; color; runtime resize after host boot.
pub fn user_root_slots_from_entries(entries: &[SceneEntry]) -> Result<usize, UserRootSlotsError> {
    let mut seen = [false; USER_ROOT_SLOTS_MAX];
    let mut max_slot: Option<usize> = None;
    for entry in entries {
        if !scene_entry_is_user_root(entry.entity_id) {
            continue;
        }
        let slot = (entry.entity_id - USER_ROOT_ENTITY_FIRST) as usize;
        if seen[slot] {
            return Err(UserRootSlotsError::Duplicate {
                entity_id: entry.entity_id,
            });
        }
        seen[slot] = true;
        max_slot = Some(max_slot.map_or(slot, |m| m.max(slot)));
    }
    let Some(max_slot) = max_slot else {
        return Err(UserRootSlotsError::Empty);
    };
    let n = max_slot + 1;
    for slot in 0..n {
        if !seen[slot] {
            return Err(UserRootSlotsError::Gap {
                missing_entity_id: USER_ROOT_ENTITY_FIRST + slot as u32,
            });
        }
    }
    Ok(n)
}

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
        let mut upsert = entity_trs(
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
        );
        upsert.holder = entity_holder_from_slot(0);
        let frame = RenderFrame {
            tick: 42,
            host_time_ms: 1_000,
            upserts: vec![upsert],
            removes: vec![9],
            host_sig: vec![],
            sim_tick: 84,
            redundancy: 0,
            acked_intent_seq: vec![3, 0, 7],
            scene_rev: 11,
        };
        let bytes = frame.encode_to_vec();
        let decoded = RenderFrame::decode(bytes.as_slice()).expect("decode");
        assert_eq!(decoded.tick, 42);
        assert_eq!(decoded.sim_tick, 84);
        assert_eq!(decoded.redundancy, 0);
        assert_eq!(decoded.acked_intent_seq, vec![3, 0, 7]);
        assert_eq!(decoded.scene_rev, 11);
        assert_eq!(decoded.upserts.len(), 1);
        assert!((decoded.upserts[0].x - 1.5).abs() < f64::EPSILON);
        assert!((decoded.upserts[0].qw - 0.7071067811865476).abs() < 1e-9);
        assert!((decoded.upserts[0].sx - 2.0).abs() < f64::EPSILON);
        assert_eq!(decoded.upserts[0].holder, 1);
        assert_eq!(entity_holder_slot(decoded.upserts[0].holder), Some(0));
        assert_eq!(decoded.removes, vec![9]);
    }

    #[test]
    fn release_velocity_clamped() {
        let (lin, ang) = clamp_release_velocity(
            [100.0, 0.0, 0.0],
            [0.0, 80.0, 0.0],
            RELEASE_LINEAR_SPEED_MAX,
            RELEASE_ANGULAR_SPEED_MAX,
        );
        assert!((lin[0] - RELEASE_LINEAR_SPEED_MAX).abs() < 1e-5);
        assert!(lin[1].abs() < 1e-6 && lin[2].abs() < 1e-6);
        assert!((ang[1] - RELEASE_ANGULAR_SPEED_MAX).abs() < 1e-5);

        let (bad, _) = clamp_release_velocity(
            [f32::NAN, 1.0, 0.0],
            [0.0, 0.0, 0.0],
            RELEASE_LINEAR_SPEED_MAX,
            RELEASE_ANGULAR_SPEED_MAX,
        );
        assert_eq!(bad, [0.0, 0.0, 0.0]);
    }

    #[test]
    fn entity_holder_wire_roundtrip() {
        assert_eq!(entity_holder_from_slot(0), 1);
        assert_eq!(entity_holder_from_slot(31), 32);
        assert_eq!(entity_holder_from_slot(255), ENTITY_HOLDER_NONE);
        assert_eq!(entity_holder_slot(0), None);
        assert_eq!(entity_holder_slot(1), Some(0));
        assert_eq!(entity_holder_slot(33), None);
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
                    parent_entity_id: 0,
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
                    parent_entity_id: 0,
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
            release_vx: 1.5,
            release_vy: 0.0,
            release_vz: -0.25,
            release_wx: 0.1,
            release_wy: 0.0,
            release_wz: 0.0,
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
        assert!((decoded.release_vx - 1.5).abs() < f32::EPSILON);
        assert!((decoded.release_vz - -0.25).abs() < f32::EPSILON);
        assert!((decoded.release_wx - 0.1).abs() < f32::EPSILON);
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
            ..Default::default()
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
                    entity_id: USER_ROOT_ENTITY_FIRST,
                },
                PresenceAssignment {
                    sub: "bob-uuid".into(),
                    entity_id: USER_ROOT_ENTITY_FIRST + 1,
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
        assert_eq!(decoded.assignments[0].entity_id, USER_ROOT_ENTITY_FIRST);
        assert_eq!(decoded.assignments[1].sub, "bob-uuid");
        assert_eq!(decoded.protocol_pin, PROTOCOL_PIN);
        assert_eq!(decoded.host_generation, 1);
    }

    #[test]
    fn cell_id_segment_roundtrip_and_negatives() {
        assert_eq!(format_cell_id_segment(0, 0, 0), "0_0_0");
        assert_eq!(format_cell_id_segment(-1, 2, 0), "-1_2_0");
        assert_eq!(format_cell_id_segment(-1, -2, 0), "-1_-2_0");
        assert_eq!(parse_cell_id_segment("0_0_0").unwrap(), [0, 0, 0]);
        assert_eq!(parse_cell_id_segment("-1_2_0").unwrap(), [-1, 2, 0]);
        assert_eq!(parse_cell_id_segment("-1_-2_0").unwrap(), [-1, -2, 0]);
        assert_eq!(
            parse_cell_id_segment("").unwrap_err(),
            CellIdSegmentError::Empty
        );
        assert_eq!(
            parse_cell_id_segment("0/0/0").unwrap_err(),
            CellIdSegmentError::ContainsSlash
        );
        assert_eq!(
            parse_cell_id_segment("0_0").unwrap_err(),
            CellIdSegmentError::WrongPartCount
        );
        assert_eq!(
            parse_cell_id_segment("0_0_0_1").unwrap_err(),
            CellIdSegmentError::WrongPartCount
        );
        assert_eq!(
            parse_cell_id_segment("a_0_0").unwrap_err(),
            CellIdSegmentError::InvalidInteger
        );
        assert_eq!(
            parse_cell_id_segment("_0_0").unwrap_err(),
            CellIdSegmentError::InvalidInteger
        );
    }

    #[test]
    fn key_helpers() {
        let cap = LOCAL_WIRE_CAP;
        assert_eq!(frame_key("local", cap), FRAME_KEY_LOCAL);
        assert_eq!(
            frame_cell_key("local", cap, 0, 0, 0),
            FRAME_CELL_KEY_LOCAL_ORIGIN
        );
        assert_ne!(
            frame_cell_key("local", cap, 0, 0, 0),
            frame_key("local", cap)
        );
        assert_eq!(
            frame_cell_key("alpha", cap, -1, -2, 3),
            format!("alchemy/dev/room/alpha/{cap}/frame/cell/-1_-2_3")
        );
        assert_eq!(
            frame_lod_key("local", cap, 0, 0, 0),
            FRAME_LOD_KEY_LOCAL_ORIGIN
        );
        assert_ne!(
            frame_lod_key("local", cap, 0, 0, 0),
            frame_cell_key("local", cap, 0, 0, 0)
        );
        assert_eq!(
            frame_lod_key("alpha", cap, -1, -2, 3),
            format!("alchemy/dev/room/alpha/{cap}/frame/lod/-1_-2_3")
        );
        assert_eq!(
            frame_lod_key_from_cell("local", cap, 0, 0, 0),
            FRAME_LOD_KEY_LOCAL_ORIGIN
        );
        assert_eq!(
            frame_lod_key_from_cell("local", cap, 2, 5, -1),
            frame_lod_key("local", cap, 0, 1, -1)
        );
        assert_eq!(input_key("local", cap), INPUT_KEY_LOCAL);
        assert_eq!(manifest_key("local", cap), MANIFEST_KEY_LOCAL);
        assert_eq!(host_control_key("local", cap), HOST_CONTROL_KEY_LOCAL);
        assert_eq!(inventory_spawn_key("local", cap), INVENTORY_SPAWN_KEY_LOCAL);
        assert_eq!(
            inventory_despawn_key("local", cap),
            INVENTORY_DESPAWN_KEY_LOCAL
        );
        assert_eq!(presence_key("local", cap), PRESENCE_KEY_LOCAL);
        assert_eq!(snapshot_key("local", cap), SNAPSHOT_KEY_LOCAL);
        assert_eq!(sim_state_key("local", cap), SIM_STATE_KEY_LOCAL);
        assert_eq!(edit_key("local", cap), EDIT_KEY_LOCAL);
        assert_eq!(
            presence_key("alpha", cap),
            format!("alchemy/dev/room/alpha/{cap}/presence")
        );
        assert_ne!(
            presence_key("alpha", "cap-a"),
            presence_key("alpha", "cap-b")
        );
        assert_eq!(
            liveliness_key("local", cap, "alice-uuid", "jti-1"),
            format!("{LIVELINESS_KEY_LOCAL_PREFIX}/alice-uuid/jti-1")
        );
        assert_eq!(
            liveliness_wildcard("alpha", cap),
            format!("alchemy/dev/room/alpha/{cap}/alive/**")
        );
        assert_eq!(
            liveliness_binding_from_key(&format!(
                "alchemy/dev/room/local/{cap}/alive/alice-uuid/jti-1"
            )),
            Some(("alice-uuid", "jti-1"))
        );
        assert_eq!(
            liveliness_binding_from_key(&format!("alchemy/dev/room/local/{cap}/alive/alice-uuid")),
            None
        );
        assert_eq!(
            liveliness_binding_from_key(&format!("alchemy/dev/room/local/{cap}/alive/a/b/c")),
            None
        );
        assert!(validate_wire_cap(LOCAL_WIRE_CAP).is_ok());
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
    fn user_root_slots_from_entries_contiguous() {
        let roots: Vec<SceneEntry> = (0..USER_ROOT_SLOTS_MAX)
            .map(|i| SceneEntry {
                entity_id: USER_ROOT_ENTITY_FIRST + i as u32,
                ..Default::default()
            })
            .collect();
        assert_eq!(
            user_root_slots_from_entries(&roots).unwrap(),
            USER_ROOT_SLOTS_MAX
        );
        assert_eq!(user_root_slots_from_entries(&roots[..2]).unwrap(), 2);
        // 5+ rooms are Manifest-width within the DoS ceiling (x009b / P2-13).
        assert_eq!(user_root_slots_from_entries(&roots[..5]).unwrap(), 5);
        assert_eq!(
            user_root_slots_from_entries(&[]).unwrap_err(),
            UserRootSlotsError::Empty
        );
        let gap = vec![
            SceneEntry {
                entity_id: USER_ROOT_ENTITY_FIRST,
                ..Default::default()
            },
            SceneEntry {
                entity_id: USER_ROOT_ENTITY_FIRST + 2,
                ..Default::default()
            },
        ];
        assert_eq!(
            user_root_slots_from_entries(&gap).unwrap_err(),
            UserRootSlotsError::Gap {
                missing_entity_id: USER_ROOT_ENTITY_FIRST + 1
            }
        );
        let dup = vec![
            SceneEntry {
                entity_id: USER_ROOT_ENTITY_FIRST,
                ..Default::default()
            },
            SceneEntry {
                entity_id: USER_ROOT_ENTITY_FIRST,
                ..Default::default()
            },
        ];
        assert_eq!(
            user_root_slots_from_entries(&dup).unwrap_err(),
            UserRootSlotsError::Duplicate {
                entity_id: USER_ROOT_ENTITY_FIRST
            }
        );
        // Legacy low ids are outside the band (pre-x009b DemoScene 3..6).
        assert!(!scene_entry_is_user_root(3));
        assert!(!scene_entry_is_user_root(6));
        assert!(scene_entry_is_user_root(USER_ROOT_ENTITY_FIRST));
        assert!(scene_entry_is_user_root(user_root_entity_last()));
        assert!(!scene_entry_is_user_root(user_root_entity_last() + 1));
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
    #[allow(deprecated)]
    fn cell_frame_sub_defaults_off() {
        // Legacy API: historical default off; x00c5 mode selection uses room_frame_*.
        assert!(!cell_frame_sub_enabled_from(None));
        assert!(!cell_frame_sub_enabled_from(Some("0")));
        assert!(cell_frame_sub_enabled_from(Some("1")));
        assert!(cell_frame_sub_enabled_from(Some("true")));
    }

    #[test]
    fn room_frame_sub_defaults_off() {
        assert!(!room_frame_sub_enabled_from(None));
        assert!(!room_frame_sub_enabled_from(Some("0")));
        assert!(!room_frame_sub_enabled_from(Some("false")));
        assert!(room_frame_sub_enabled_from(Some("1")));
        assert!(room_frame_sub_enabled_from(Some("true")));
        assert!(room_frame_sub_enabled_from(Some("YES")));
    }

    #[test]
    fn moore_neighborhood_includes_center_and_has_27() {
        let cells = moore_neighborhood_cells(0, 0, 0);
        assert_eq!(cells.len(), MOORE_NEIGHBORHOOD_LEN);
        assert!(cells.contains(&[0, 0, 0]));
        assert!(cells.contains(&[-1, -1, -1]));
        assert!(cells.contains(&[1, 1, 1]));
        let mut uniq = cells.to_vec();
        uniq.sort_unstable();
        uniq.dedup();
        assert_eq!(uniq.len(), MOORE_NEIGHBORHOOD_LEN);
    }

    #[test]
    fn frame_cell_interest_keyexpr_unions_keys() {
        let cap = LOCAL_WIRE_CAP;
        let cells = [[0, 0, 0], [1, 0, 0]];
        let ke = frame_cell_interest_keyexpr("local", cap, &cells);
        assert_eq!(
            ke,
            format!(
                "{}|{}",
                frame_cell_key("local", cap, 0, 0, 0),
                frame_cell_key("local", cap, 1, 0, 0)
            )
        );
        assert!(!ke.contains("/frame/**"));
        assert!(ke.contains("/frame/cell/0_0_0"));
    }

    #[test]
    fn frame_cell_wildcard_key_is_acl_aligned() {
        let cap = LOCAL_WIRE_CAP;
        let ke = frame_cell_wildcard_key("local", cap);
        assert_eq!(ke, format!("{}/cell/*", frame_key("local", cap)));
        assert!(ke.ends_with("/frame/cell/*"));
        assert!(!ke.contains('|'));
        assert!(!ke.contains("/frame/**"));
    }

    #[test]
    fn cell_to_lod_cell_stride_and_negatives() {
        assert_eq!(LOD_CELL_STRIDE, 3);
        assert_eq!(cell_to_lod_cell(0, 0, 0), [0, 0, 0]);
        assert_eq!(cell_to_lod_cell(2, 2, 2), [0, 0, 0]);
        assert_eq!(cell_to_lod_cell(3, 0, 0), [1, 0, 0]);
        assert_eq!(cell_to_lod_cell(5, -1, 8), [1, -1, 2]);
        // Floor division: truncating toward zero would wrongly map -1 → 0.
        assert_eq!(cell_to_lod_cell(-1, 0, 0), [-1, 0, 0]);
        assert_eq!(cell_to_lod_cell(-3, -4, 0), [-1, -2, 0]);
        assert_eq!(cell_to_lod_cell(-6, 3, -3), [-2, 1, -1]);
    }

    #[test]
    fn frame_lod_wildcard_key_is_acl_aligned() {
        let cap = LOCAL_WIRE_CAP;
        let ke = frame_lod_wildcard_key("local", cap);
        assert_eq!(ke, format!("{}/lod/*", frame_key("local", cap)));
        assert!(ke.ends_with("/frame/lod/*"));
        assert!(!ke.contains('|'));
        assert!(!ke.contains("/frame/**"));
        assert!(!ke.contains("/frame/cell/"));
    }

    #[test]
    fn lod_moore_reuses_neighborhood_helper() {
        let [lx, ly, lz] = cell_to_lod_cell(4, -2, 7);
        let cells = moore_neighborhood_cells(lx, ly, lz);
        assert_eq!(cells.len(), MOORE_NEIGHBORHOOD_LEN);
        assert!(cells.contains(&[lx, ly, lz]));
    }

    #[test]
    fn frame_cell_interest_keyexpr_empty_and_dedup() {
        let cap = LOCAL_WIRE_CAP;
        assert_eq!(frame_cell_interest_keyexpr("local", cap, &[]), "");
        let dup = [[0, 0, 0], [0, 0, 0], [1, 0, 0]];
        let ke = frame_cell_interest_keyexpr("local", cap, &dup);
        assert_eq!(
            ke,
            format!(
                "{}|{}",
                frame_cell_key("local", cap, 0, 0, 0),
                frame_cell_key("local", cap, 1, 0, 0)
            )
        );
        // saturating edge: neighborhood around i32::MAX collapses; keyexpr stays unique.
        let edge = moore_neighborhood_cells(i32::MAX, i32::MAX, i32::MAX);
        let edge_ke = frame_cell_interest_keyexpr("local", cap, &edge);
        assert!(!edge_ke.is_empty());
        let mut parts: Vec<&str> = edge_ke.split('|').collect();
        assert!(parts.len() < MOORE_NEIGHBORHOOD_LEN);
        let before = parts.len();
        parts.sort_unstable();
        parts.dedup();
        assert_eq!(parts.len(), before);
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

    /// x00c0: Room may re-apply same tick only when previous commit was partial cell.
    #[test]
    fn decide_room_frame_tick_overrides_partial_cell() {
        assert_eq!(
            decide_room_frame_tick(Some(10), 10, FRAME_TICK_GAP_RESYNC, false),
            FrameTickDecision::Apply
        );
        assert_eq!(
            decide_room_frame_tick(Some(10), 10, FRAME_TICK_GAP_RESYNC, true),
            FrameTickDecision::DropStale
        );
        assert_eq!(
            decide_room_frame_tick(Some(10), 9, FRAME_TICK_GAP_RESYNC, false),
            FrameTickDecision::DropStale
        );
        assert_eq!(
            decide_room_frame_tick(Some(10), 11, FRAME_TICK_GAP_RESYNC, true),
            FrameTickDecision::Apply
        );
    }

    /// x00c3: cell may re-apply same tick when previous commit was also partial cell.
    #[test]
    fn decide_cell_frame_tick_merges_partial_cell() {
        assert_eq!(
            decide_cell_frame_tick(Some(10), 10, FRAME_TICK_GAP_RESYNC, false),
            FrameTickDecision::Apply
        );
        assert_eq!(
            decide_cell_frame_tick(Some(10), 10, FRAME_TICK_GAP_RESYNC, true),
            FrameTickDecision::DropStale
        );
        assert_eq!(
            decide_cell_frame_tick(Some(10), 9, FRAME_TICK_GAP_RESYNC, false),
            FrameTickDecision::DropStale
        );
        assert_eq!(
            decide_cell_frame_tick(Some(10), 11, FRAME_TICK_GAP_RESYNC, false),
            FrameTickDecision::Apply
        );
        assert_eq!(
            decide_cell_frame_tick(Some(10), 12, FRAME_TICK_GAP_RESYNC, false),
            FrameTickDecision::ResyncSnapshot
        );
    }
}
