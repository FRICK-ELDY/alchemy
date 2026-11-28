//! Shared scene GPU (Coin + floor + grabbable boxes + remote user_roots).
//!
//! MoveIntent carries Grab hold fields for x0011 (host kinematic follow).
//! `CameraLook` is Desktop-local (x0012); not published on Zenoh.
//!
//! Guarantees: draw with caller-supplied view/proj; grabbable boxes from Manifest (x0027);
//! remote roots in the protocol user_root band (x0015 / x009b);
//! remounted world GLB instanced at each Manifest mesh entry pose (take-out visible; same GPU
//! asset for all blake3 until multi-GLB; first [`MESH_DRAW_SLOTS`] poses only);
//! [`SharedState::asset_status`] sticky Home overlay for auth asset failure (x0048);
//! empty shell (`index_count==0`) skips floor+world mesh (MainHub; x007d);
//! [`SceneGpu::replace_world_mesh`] + [`SharedState::pending_world_mesh`] for Manifest remount;
//! Epoch Prev/Curr GPU interpolate before the world pass (x0090); mesh/box **display TRS** from
//! `epoch_out` (x0091; full active pack; no readback);
//! [`SharedState::begin_display_frame`] freezes α/PrevCurr/pose cache once per frame; Grab /
//! coin Y / GPU upload / FPS eye / XR floor anchor all read that snap (x002b / x0091 / x0092).
//! Non-guarantees: per-blake3 multi-GLB download; mesh draw beyond [`MESH_DRAW_SLOTS`];
//! color on tape (height gradient uses display Y when mirrored); TPS / Live camera modes;
//! XR swapchain ownership, system_ui layout, avatar GLB.
//! Abolished: Curr-only `EntityPose` as authority for on-screen draw/Grab/eye TRS.
//! Leave / liveliness token ownership lives here; put/undeclare is `room::leave_room` (re-exported as `crate::leave_room`).

use std::collections::{HashMap, HashSet};
use std::sync::atomic::{AtomicBool, AtomicU32, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

/// Manifest cuboid defs shared by draw / Grab (Arc swap on Manifest apply).
#[derive(Clone, Default)]
pub struct BoxDefs {
    pub grabbable: HashSet<u32>,
    pub half_extents: HashMap<u32, f32>,
}

use winit::window::Window;

use anyhow::{Context, Result};
use bytemuck::{Pod, Zeroable};
use glam::{Mat4, Vec3};
use wgpu::util::DeviceExt;

use alchemy_protocol::scene_entry_is_user_root;

use crate::auth::AuthGate;
use crate::mesh::CpuMesh;

/// Fallback mesh entity before Manifest arrives (fixture / smoke).
pub use alchemy_shared_sim::demo::DEMO_MESH_ENTITY_ID as MESH_ENTITY_ID;
/// Sentinel: Presence has not assigned a local user_root yet.
pub const LOCAL_USER_ROOT_UNSET: u32 = 0;
/// Max cuboid draw UBOs (demo has 9; headroom for Manifest-driven scenes).
pub const BOX_DRAW_SLOTS: usize = 16;
/// Max remounted-GLB instances (primary coin + take-out meshes; x006d / x007f).
/// Match [`BOX_DRAW_SLOTS`] headroom so multi-child Prefab + DemoScene coin fit before clip.
pub const MESH_DRAW_SLOTS: usize = 16;
/// Default half-extent when Manifest omits per-entity size.
pub use alchemy_shared_sim::DEFAULT_BOX_HALF_EXTENT as BOX_HALF_EXTENT;
/// Placeholder size for remote player (meters half-extent).
pub const REMOTE_ROOT_HALF_EXTENT: f32 = 0.25;
/// Guest Room Token probe cadence for `room_key_epoch` (x007e; matches host ≈15s sync).
pub const ROOM_KEY_EPOCH_PROBE: Duration = Duration::from_secs(15);
/// Pending AEAD grace before hard KeyStale (x00d5; ≈ host `DEFAULT_PREV_GRACE` / Token TTL).
pub const PENDING_ROOM_AEAD_GRACE: Duration = Duration::from_secs(300);

/// Shared Room AEAD key bytes (x0069 / x00d5).
pub type RoomAeadKey = Arc<[u8; alchemy_protocol::wire_aead::ROOM_KEY_LEN]>;
/// Dual-open snapshot: `(current_key, current_epoch, prev, bus_generation)` (x00d5).
pub type RoomAeadOpenView = (RoomAeadKey, u64, Option<(RoomAeadKey, u64)>, u64);

/// Stashed next `(room_key, epoch)` while live AEAD still opens residual old ciphertext (x00d5).
#[derive(Clone, Debug)]
pub struct PendingRoomAead {
    pub key: RoomAeadKey,
    pub epoch: u64,
    pub deadline: Instant,
}

/// Decision for Token refresh vs live `(room_key, epoch)` (x007e / x00d5).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum RefreshAeadAction {
    /// Same epoch, not KeyStale — sync live bytes.
    Commit,
    /// Token epoch advanced by exactly 1 — stash pending; dual-open until Manifest commit (x00d5).
    PreparePending,
    /// Hard stop — UI must Rejoin (grace fail, already KeyStale, or epoch gap > 1).
    KeyStale,
    /// Token epoch below live — refuse (monotonic fail-closed).
    IgnoreBackward,
    /// Already KeyStale and Token epoch equals live — do not clear via commit.
    /// (Lower epochs take [`IgnoreBackward`] first.)
    IgnoreWhileStale,
}

/// Pure refresh policy (x00d5): `+1` → PreparePending; hard-stale or gap `>1` → KeyStale.
pub(crate) fn refresh_aead_action(
    live_epoch: u64,
    token_epoch: u64,
    key_stale: bool,
) -> RefreshAeadAction {
    if token_epoch > live_epoch {
        if key_stale || token_epoch > live_epoch.saturating_add(1) {
            // Hard-stale, or skipped a generation (host current+prev only) — Rejoin.
            RefreshAeadAction::KeyStale
        } else {
            RefreshAeadAction::PreparePending
        }
    } else if token_epoch < live_epoch {
        RefreshAeadAction::IgnoreBackward
    } else if key_stale {
        RefreshAeadAction::IgnoreWhileStale
    } else {
        RefreshAeadAction::Commit
    }
}

/// Under `room_key` → `pending` lock order: install pending into live only when bus is up
/// and live key still present. Clears pending **after** copying the Arc (x00d5).
pub(crate) fn commit_pending_into_live(
    live_key: &mut Option<Arc<[u8; alchemy_protocol::wire_aead::ROOM_KEY_LEN]>>,
    pending: &mut Option<PendingRoomAead>,
    opened_epoch: u64,
    bus_started: bool,
) -> Option<(Arc<[u8; alchemy_protocol::wire_aead::ROOM_KEY_LEN]>, u64)> {
    if !bus_started || live_key.is_none() {
        return None;
    }
    let p = pending.as_ref()?;
    if p.epoch != opened_epoch {
        return None;
    }
    let key = Arc::clone(&p.key);
    let epoch = p.epoch;
    *live_key = Some(Arc::clone(&key));
    *pending = None;
    Some((key, epoch))
}

/// Under pending lock: if deadline passed, take pending and return its epoch (x00d5).
pub(crate) fn take_pending_if_grace_expired(
    pending: &mut Option<PendingRoomAead>,
    now: Instant,
) -> Option<u64> {
    match pending.as_ref() {
        Some(p) if now >= p.deadline => {
            let epoch = p.epoch;
            *pending = None;
            Some(epoch)
        }
        _ => None,
    }
}

/// Soft-TTL / Join live commit: drop pending unless it is **ahead** of `commit_epoch`
/// (delayed same-epoch response must not wipe a newer seamless stash · x00d5 §8).
pub(crate) fn clear_pending_unless_newer(pending: &mut Option<PendingRoomAead>, commit_epoch: u64) {
    let keep_newer = pending.as_ref().is_some_and(|p| p.epoch > commit_epoch);
    if !keep_newer {
        *pending = None;
    }
}

pub fn is_user_root_entity(id: u32) -> bool {
    scene_entry_is_user_root(id)
}
/// Half-extent of Desktop floor (meters). Larger so loco is obvious.
const FLOOR_EXTENT: f32 = 20.0;
/// Default eye height if user_root not yet received.
const DEFAULT_EYE_Y: f32 = 1.6;

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct EntityPose {
    pub x: f32,
    pub y: f32,
    pub z: f32,
    /// Unit quaternion xyzw (identity when unset).
    pub qx: f32,
    pub qy: f32,
    pub qz: f32,
    pub qw: f32,
    /// Frame scale (multiplies Manifest half_extent / mesh visual size).
    pub sx: f32,
    pub sy: f32,
    pub sz: f32,
    pub r: f32,
    pub g: f32,
    pub b: f32,
    pub a: f32,
}

impl Default for EntityPose {
    fn default() -> Self {
        Self {
            x: 0.0,
            y: 0.0,
            z: 0.0,
            qx: 0.0,
            qy: 0.0,
            qz: 0.0,
            qw: 1.0,
            sx: 1.0,
            sy: 1.0,
            sz: 1.0,
            r: 1.0,
            g: 1.0,
            b: 1.0,
            a: 1.0,
        }
    }
}

impl EntityPose {
    pub fn position(self) -> Vec3 {
        Vec3::new(self.x, self.y, self.z)
    }

    /// Wire quat → glam; zero-length (proto3 unset) → identity.
    pub fn rotation(self) -> glam::Quat {
        let q = glam::Quat::from_xyzw(self.qx, self.qy, self.qz, self.qw);
        if q.length_squared() < 1e-12 {
            glam::Quat::IDENTITY
        } else {
            q.normalize()
        }
    }

    /// Wire scale; all-zero (proto3 unset) → (1,1,1).
    pub fn scale(self) -> Vec3 {
        let s = Vec3::new(self.sx, self.sy, self.sz);
        if s.length_squared() < 1e-12 {
            Vec3::ONE
        } else {
            s
        }
    }

    pub fn color(self) -> [f32; 4] {
        [self.r, self.g, self.b, self.a]
    }

    pub fn is_user_root(id: u32) -> bool {
        is_user_root_entity(id)
    }
}

/// Desktop look (client-local). Position comes from host user_root.
#[derive(Clone, Copy, Debug)]
pub struct CameraLook {
    pub yaw: f32,
    pub pitch: f32,
}

impl Default for CameraLook {
    fn default() -> Self {
        Self {
            yaw: 0.0,
            pitch: 0.0,
        }
    }
}

/// Shared locomotion + Grab hold intent published on Zenoh (mode-agnostic).
#[derive(Clone, Copy, Debug, Default)]
pub struct MoveIntent {
    pub move_x: f32,
    pub move_y: f32,
    pub move_z: f32,
    pub buttons: u32,
    /// When true (dash open), publisher forces move zeros (Grab also cleared).
    pub suppress: bool,
    /// Grabbable entity while GRAB held (`0` = none). XR grip or Desktop RMB (x0013).
    pub grab_entity_id: u32,
    /// World-space aim origin (meters): XR controller aim / Desktop eye (user_root).
    pub aim_x: f32,
    pub aim_y: f32,
    pub aim_z: f32,
}

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct MeshVertex {
    position: [f32; 3],
}

/// World-pass uniforms (x0091). `pack_index == PACK_INDEX_MODEL_OVERRIDE` → `model_override`.
#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
pub struct Uniforms {
    pub view_proj: [[f32; 4]; 4],
    pub color: [f32; 4],
    pub scale_mul: [f32; 3],
    pub pack_index: u32,
    pub extra_rot: [f32; 4],
    pub model_override: [[f32; 4]; 4],
}

impl Uniforms {
    fn identity_rot() -> [f32; 4] {
        [0.0, 0.0, 0.0, 1.0]
    }

    /// Floor / CPU-TRS fallback: full model in `model_override`.
    fn with_model(view_proj: Mat4, model: Mat4, color: [f32; 4]) -> Self {
        Self {
            view_proj: view_proj.to_cols_array_2d(),
            color,
            scale_mul: [1.0, 1.0, 1.0],
            pack_index: crate::epoch_view::PACK_INDEX_MODEL_OVERRIDE,
            extra_rot: Self::identity_rot(),
            model_override: model.to_cols_array_2d(),
        }
    }

    /// Epoch `out[pack_index]` TRS × `scale_mul` × `extra_rot`.
    fn with_epoch(
        view_proj: Mat4,
        pack_index: u32,
        scale_mul: Vec3,
        extra_rot: glam::Quat,
        color: [f32; 4],
    ) -> Self {
        Self {
            view_proj: view_proj.to_cols_array_2d(),
            color,
            scale_mul: scale_mul.to_array(),
            pack_index,
            extra_rot: [extra_rot.x, extra_rot.y, extra_rot.z, extra_rot.w],
            model_override: Mat4::IDENTITY.to_cols_array_2d(),
        }
    }
}

/// Pose table + tick gate under one lock (x0023: snapshot/delta race-free).
#[derive(Default)]
pub struct WorldView {
    pub last_frame_tick: Option<u64>,
    /// True when [`Self::last_frame_tick`] was committed by Room `…/frame` or snapshot (x00c0).
    /// False after a partial cell apply so same-tick Room may override under 併読,
    /// and additional cell samples may merge (x00c3).
    pub last_frame_from_room: bool,
    /// Entity ids that survived a cell upsert on the current [`Self::last_frame_tick`]
    /// (x00c3). Later same-tick removes skip a guarded id only when **that id** has no
    /// upsert in the same frame (per-id remove-only → cross-cell upsert wins). If the
    /// frame also upserts the id, within-frame remove still wins.
    /// Cleared on tick advance / Room / snapshot. Opt-in hot path only; grows with distinct
    /// dirty upsert ids per tick (no hard cap — reuse via clear at tick boundary; cell 既定化
    /// 時は dirty 予算と合わせて見直す).
    pub cell_tick_upsert_guard: std::collections::HashSet<u32>,
    pub entities: std::collections::HashMap<u32, EntityPose>,
}

/// Side effect for [`SharedState::try_apply_world_remount`] (x007d).
#[derive(Debug, Clone)]
pub enum RemountedBlake3Update {
    Unchanged,
    Set(Option<String>),
}

/// Side effect for [`SharedState::try_apply_world_remount`] (x007d).
#[derive(Debug, Clone)]
pub enum AssetStatusUpdate {
    Unchanged,
    Clear,
    Set(String),
}

pub struct SharedState {
    /// Local camera / aim follows this host entity (`LOCAL_USER_ROOT_UNSET` until Presence).
    pub local_user_root: AtomicU32,
    /// Active `room_id` for Zenoh keys (x0017). Mutex so Worlds catalog can retarget (x007c).
    pub room_id: Mutex<Arc<str>>,
    /// Zenoh key channel secret from Room Token (x004e); `None` until join.
    pub wire_cap: Mutex<Option<Arc<str>>>,
    /// Room AEAD key from Room Token (x0069); `None` until join.
    pub room_key: Mutex<Option<Arc<[u8; alchemy_protocol::wire_aead::ROOM_KEY_LEN]>>>,
    /// Room key epoch from Token (x007e); paired with `room_key` for AEAD AAD.
    pub room_key_epoch: AtomicU64,
    /// Hard KeyStale — UI must Rejoin / Leave (x007e / x00d5 grace fail).
    /// Soft epoch advance uses [`Self::pending_room_aead`] instead (seamless path).
    pub key_stale: AtomicBool,
    /// Next `(key, epoch)` while live still opens residual old ciphertext (x00d5).
    pub pending_room_aead: Mutex<Option<PendingRoomAead>>,
    /// Wire AEAD generation — bumped on pending commit / KeyStale / Leave (x00d5).
    pub bus_generation: AtomicU64,
    /// Next guest Room Token probe for epoch detection (x007e; ≈ host 15s sync).
    pub room_key_epoch_probe_at: Mutex<Option<Instant>>,
    /// Rate-limit AEAD-fail → Token refresh nudges.
    pub aead_key_refresh_nudge_at: Mutex<Option<Instant>>,
    /// Once true, manifest/frame/presence/input subscribers are running (x004e).
    /// Cleared on Leave and before re-attach after Zenoh reopen.
    pub room_bus_started: std::sync::atomic::AtomicBool,
    /// One-shot: allocate wrote new `ZENOH_TLS_*`; next `bind_room_after_join` must reopen.
    /// Env alone must not trigger reopen (would orphan subscribers on an already-running bus).
    pub mtls_reopen_pending: std::sync::atomic::AtomicBool,
    /// World JWKS cache (retargeted when onboarding switches operator).
    pub world_jwks: Arc<alchemy_client_trust::WorldJwks>,
    /// World JWKS verifiers for Manifest path (needed for late Home→Join bus start).
    pub manifest_seal: Arc<alchemy_client_trust::ManifestSealVerifier>,
    pub host_attest: Arc<alchemy_client_trust::HostAttestationVerifier>,
    /// Mutable so onboarding Step1 can switch Official / Custom without restart.
    pub auth_url: Mutex<String>,
    pub world_url: Mutex<String>,
    pub assets_url: Mutex<String>,
    /// Saved for join / prefs; Zenoh open still happens at join time.
    pub zenoh_connect: Mutex<String>,
    /// Auth + Room Token gate (x0018). Intent publishes only when `room_token` is set.
    pub auth: Mutex<AuthGate>,
    /// Ed25519 Intent session while joined (x0023).
    pub intent_session: Mutex<Option<crate::intent_session::IntentSession>>,
    /// Host-generation rotate: new key held here until Room Token arrives (x003e).
    /// Live publisher keeps `intent_session` (old key + old Token) until atomic commit.
    /// Install / clear only under lock order auth → intent_session → pending.
    pub pending_intent_session: Mutex<Option<crate::intent_session::IntentSession>>,
    /// Earliest time + last wait for pending Token fetch backoff (world outage).
    pub pending_retry: Mutex<Option<(Instant, Duration)>>,
    /// Host Frame verify key from Manifest `host_pub` (x0023).
    pub host_pub: Mutex<Option<Vec<u8>>>,
    /// Last accepted Manifest `(host_generation, revision)` watermark (x003a).
    pub manifest_gen_rev: Mutex<Option<alchemy_protocol::generation_gate::GenRev>>,
    /// Monotonic tip of **accepted** `EditCommit.scene_rev` (host live tip; x00b3).
    /// Used as `EditProposal.base_scene_rev` — not overlay/Manifest revision; rejects do not update.
    pub last_edit_scene_rev: Mutex<Option<u64>>,
    /// Pending Room Save `EditProposal.op_id` awaiting `EditCommit` (x00b7).
    pub pending_save_op_id: Mutex<Option<u64>>,
    /// Latest Save status for Dash (`saved` / `forbidden` / `conflict` / …).
    pub room_save_status: Mutex<Option<String>>,
    /// Last accepted Manifest `world_seal` for PlaceRead issuance (x006c).
    pub last_world_seal: Mutex<Option<String>>,
    /// Last accepted Presence watermark (x003a); generation must match Manifest.
    pub presence_gen_rev: Mutex<Option<alchemy_protocol::generation_gate::GenRev>>,
    /// Zenoh liveliness token while joined (x0022 `sub`+`jti`). Drop/undeclare on leave.
    pub liveliness: Mutex<Option<zenoh::liveliness::LivelinessToken>>,
    /// Previous liveliness key retained one Intent put after refresh rebind (x0022).
    pub liveliness_prev: Mutex<Option<zenoh::liveliness::LivelinessToken>>,
    /// Entity poses + last applied frame tick (single mutex).
    pub world: Mutex<WorldView>,
    /// Wall time of last accepted host signal (Frame / Presence / Manifest).
    /// Empty-delta hosts do not advance `net_tick` (x0024); stall UX must not rely on tick alone.
    pub last_host_alive_at: Mutex<Option<Instant>>,
    /// Optional localhost Prometheus metrics (x0049); always present, server off unless env.
    pub metrics: Arc<crate::metrics::ClientMetrics>,
    /// Manifest cuboid defs (Grab flags + half_extents); Arc-swapped on apply (x0027).
    pub box_defs: Mutex<Arc<BoxDefs>>,
    /// Primary mesh entity id from Manifest (`blake3` entry); 0 until set.
    pub mesh_entity_id: AtomicU32,
    /// Scene mesh blake3 map for Inventory put-back (x006e); updated on Manifest.
    pub scene_meshes: Mutex<Vec<(u32, String)>>,
    /// Authenticated asset-load failure message for dash Home status (x0048).
    /// `None` when mesh came from assets GET or empty shell / no failure.
    pub asset_status: Mutex<Option<String>>,
    /// Latest world mesh for GPU remount (x007d). Written by remount worker; surfaces poll by epoch.
    pub pending_world_mesh: Mutex<Option<CpuMesh>>,
    /// Bumped on each [`Self::queue_world_mesh`] so Desktop + XR companion both remount.
    pub pending_mesh_epoch: AtomicU64,
    /// Last blake3 hex successfully remounted so Manifest can skip same-hash resends.
    pub remounted_blake3: Mutex<Option<String>>,
    /// Bumped on Leave and host-generation reset so in-flight remount GET results are discarded (x007d).
    pub remount_generation: AtomicU64,
    /// Bumped on Logout / Leave discard so in-flight auth-http workers abort before
    /// `ensure_running` / `join_room_with_access` commit (x007c).
    pub auth_work_epoch: AtomicU64,
    pub look: Mutex<CameraLook>,
    pub intent: Mutex<MoveIntent>,
    /// XR ↔ Desktop present ownership (x0053). Desktop-only keeps `vr_active=false`.
    pub presentation: crate::surface::PresentationGate,
    /// Dash logical state shared Desktop ↔ XR (x0053). Layout/hover stay per-surface.
    pub dash: Mutex<crate::system_ui::SharedDashStore>,
    /// XR → Desktop Inventory ops with selection payloads (x007f). Desktop owns HTTP/Zenoh.
    pub inventory_peer_actions: Mutex<Vec<crate::system_ui::InventoryPeerRequest>>,
    /// Desktop → XR Drive list + inventory status (x007f). Scene meshes stay on `scene_meshes`.
    pub inventory_mirror: Mutex<crate::system_ui::InventoryMirror>,
    /// On-demand local host-server child (Start session only; Join stays idle).
    pub host_supervisor: Arc<crate::host_supervisor::HostSupervisor>,
    /// Other Presence assignment `sub`s (x0080 handover candidate list; hint only).
    pub presence_peer_subs: Mutex<Vec<String>>,
    /// Zero-Base Epoch Prev/Curr snapshots + Hierarchy / X-Ray (Ring 3 view).
    pub epoch: crate::epoch_view::EpochViewState,
    /// Local user_root / Grab prediction vs host authority (x00ab).
    pub owned_prediction: Mutex<crate::owned_prediction::OwnedPrediction>,
    /// Grab-agnostic pending local ops + Release/Acquire emptiness gate (x00e6).
    /// Display only; never mutates Published / EpochSnapshotPair.
    /// Sole mutation API: [`crate::pending_local_ops::PendingLocalGate`] methods.
    pub pending_local: crate::pending_local_ops::PendingLocalGate,
    /// FPS eye TRS frozen with [`Self::begin_display_frame`] (x0092; avoid N× lock per frame).
    /// Keyed by `local_user_root` id so Presence races cannot serve a stale anchor.
    pub(crate) eye_pose_cache: Mutex<Option<(u32, EntityPose)>>,
}

impl SharedState {
    pub fn get_auth_url(&self) -> String {
        self.auth_url
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .clone()
    }

    pub fn get_world_url(&self) -> String {
        self.world_url
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .clone()
    }

    pub fn get_assets_url(&self) -> String {
        self.assets_url
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .clone()
    }

    pub fn get_zenoh_connect(&self) -> String {
        self.zenoh_connect
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .clone()
    }

    /// Snapshot of the active room id (x0017 / x007c).
    pub fn get_room_id(&self) -> Arc<str> {
        self.room_id
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .clone()
    }

    /// First other Presence `sub` for Graceful Handover nomination (x0080).
    pub fn first_presence_peer_sub(&self) -> Option<String> {
        self.presence_peer_subs
            .lock()
            .ok()
            .and_then(|g| g.first().cloned())
    }

    /// Retarget room id + seal/attestation verifiers + process `ROOM_ID` env (x007c).
    ///
    /// HostSupervisor children inherit env, so `ROOM_ID` must match before spawn.
    /// Caller must Leave first if already joined (bus keys bind the previous id), and
    /// must restart a supervised host when the id changes (`ensure_running_for_room`).
    pub fn set_room_id(&self, room_id: Arc<str>) {
        if let Err(err) = self.manifest_seal.set_expected_room_id(room_id.as_ref()) {
            log::error!("manifest_seal retarget failed: {err:#}");
        }
        if let Err(err) = self.host_attest.set_expected_room_id(room_id.as_ref()) {
            log::error!("host_attest retarget failed: {err:#}");
        }
        std::env::set_var("ROOM_ID", room_id.as_ref());
        *self.room_id.lock().unwrap_or_else(|p| p.into_inner()) = room_id;
    }

    /// Apply operator endpoints. Always updates SharedState + syncs process env
    /// (`AUTH_URL` / `WORLD_URL` / `ASSETS_URL` / `ZENOH_CONNECT`).
    /// Host child inherits **AUTH/WORLD/ZENOH** at spawn (`ASSETS_URL` is for this
    /// client / tooling; host-server does not read it).
    /// Already-Running host-server keeps its old env — after Official/Custom switch,
    /// stop and Host again so the child picks up new URLs (same as ZENOH retarget).
    /// UI ApplyOperatorEndpoints wins over a prior boot-time env for this process.
    pub fn apply_endpoints(&self, ep: &crate::operator_prefs::Endpoints) {
        let world_changed = self.get_world_url() != ep.world_url;
        *self.auth_url.lock().unwrap_or_else(|p| p.into_inner()) = ep.auth_url.clone();
        *self.world_url.lock().unwrap_or_else(|p| p.into_inner()) = ep.world_url.clone();
        *self.assets_url.lock().unwrap_or_else(|p| p.into_inner()) = ep.assets_url.clone();
        *self.zenoh_connect.lock().unwrap_or_else(|p| p.into_inner()) = ep.zenoh_connect.clone();
        std::env::set_var("AUTH_URL", &ep.auth_url);
        std::env::set_var("WORLD_URL", &ep.world_url);
        std::env::set_var("ASSETS_URL", &ep.assets_url);
        std::env::set_var("ZENOH_CONNECT", &ep.zenoh_connect);
        self.world_jwks.retarget_world(&ep.world_url);
        // Boot env sync often re-applies the same world_url after an initial JWKS fetch —
        // skip a second refresh so a soft-fail cannot wipe a successful first cache.
        if world_changed {
            if let Err(err) = self.world_jwks.refresh() {
                log::warn!("world JWKS refresh after operator switch: {err:#}");
            }
        }
    }

    /// Zero shared locomotion / Grab (x0053 present handoff). Call only on ownership edges.
    pub fn clear_move_intent(&self) {
        if let Ok(mut slot) = self.intent.lock() {
            *slot = MoveIntent::default();
        }
    }

    /// Sticky Home status: if an asset failure is recorded and `home_status` differs,
    /// overwrite it and return `true` (caller may open Home **once** on that edge).
    /// Already-applied overlay returns `false` so Inventory / Settings are not yanked
    /// every `about_to_wait` / redraw (x0048 / x007f handthrough).
    /// Skips while status looks busy (`…` / `...`) so "Signing in…" is not clobbered.
    pub fn overlay_asset_home_status(&self, home_status: &mut String) -> bool {
        if home_status.ends_with('…') || home_status.ends_with("...") {
            return false;
        }
        let guard = self
            .asset_status
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let Some(msg) = guard.as_ref() else {
            return false;
        };
        if home_status.as_str() == msg.as_str() {
            return false;
        }
        *home_status = msg.clone();
        true
    }

    /// Queue a world mesh for surface remount (x007d). Desktop and XR each poll via epoch.
    pub fn queue_world_mesh(&self, mesh: CpuMesh) {
        let mut guard = self
            .pending_world_mesh
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        *guard = Some(mesh);
        self.pending_mesh_epoch.fetch_add(1, Ordering::SeqCst);
    }

    /// Remount-worker apply: under `pending_world_mesh`, require matching
    /// [`Self::remount_generation`] + joined `wire_cap`, then queue mesh **and** apply
    /// `remounted_blake3` / `asset_status` before releasing the lock (x007d).
    ///
    /// Leave / host-generation invalidate under the same pending lock so a successful mesh
    /// queue cannot be followed by a stale `remounted_blake3 = Some(hex)` write-back
    /// (which would skip the same hex on re-Join while MainHub stayed empty).
    ///
    /// Lock order: `pending_world_mesh` → `wire_cap` (brief) → `remounted_blake3` /
    /// `asset_status` (brief).
    pub fn try_apply_world_remount(
        &self,
        mesh: CpuMesh,
        job_generation: u64,
        remounted: RemountedBlake3Update,
        asset_status: AssetStatusUpdate,
    ) -> bool {
        let mut pending = self
            .pending_world_mesh
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        if self.remount_generation.load(Ordering::SeqCst) != job_generation {
            return false;
        }
        if self.wire_cap().is_none() {
            return false;
        }
        *pending = Some(mesh);
        self.pending_mesh_epoch.fetch_add(1, Ordering::SeqCst);
        Self::apply_remounted_update(&self.remounted_blake3, remounted);
        Self::apply_asset_status_update(&self.asset_status, asset_status);
        true
    }

    /// Leave: bump generation, clear remounted/status, queue empty shell — all under
    /// `pending_world_mesh` so remount cannot write back `remounted_blake3` after clear.
    fn finish_leave_empty_shell(&self) {
        let mut pending = self
            .pending_world_mesh
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        self.bump_remount_generation();
        Self::apply_remounted_update(&self.remounted_blake3, RemountedBlake3Update::Set(None));
        Self::apply_asset_status_update(&self.asset_status, AssetStatusUpdate::Clear);
        *pending = Some(crate::mesh::empty_shell_mesh());
        self.pending_mesh_epoch.fetch_add(1, Ordering::SeqCst);
    }

    /// Host-generation: bump + clear `remounted_blake3` under `pending_world_mesh`.
    /// GPU mesh intentionally unchanged until the next remount GET.
    fn invalidate_remount_for_host_generation(&self) {
        let _pending = self
            .pending_world_mesh
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        self.bump_remount_generation();
        Self::apply_remounted_update(&self.remounted_blake3, RemountedBlake3Update::Set(None));
    }

    fn apply_remounted_update(slot: &Mutex<Option<String>>, update: RemountedBlake3Update) {
        match update {
            RemountedBlake3Update::Unchanged => {}
            RemountedBlake3Update::Set(hex) => {
                let mut guard = slot.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
                *guard = hex;
            }
        }
    }

    fn apply_asset_status_update(slot: &Mutex<Option<String>>, update: AssetStatusUpdate) {
        match update {
            AssetStatusUpdate::Unchanged => {}
            AssetStatusUpdate::Clear => {
                let mut guard = slot.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
                *guard = None;
            }
            AssetStatusUpdate::Set(status) => {
                let mut guard = slot.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
                *guard = Some(status);
            }
        }
    }

    /// If `applied_epoch` is behind the pending epoch, return a clone to remount and advance.
    /// Epoch and mesh are read under the same lock as [`Self::queue_world_mesh`] so a concurrent
    /// queue cannot pair an older epoch with a newer mesh.
    pub fn pending_mesh_if_newer(&self, applied_epoch: &mut u64) -> Option<CpuMesh> {
        let guard = self.pending_world_mesh.lock().ok()?;
        let epoch = self.pending_mesh_epoch.load(Ordering::SeqCst);
        if epoch == *applied_epoch {
            return None;
        }
        let mesh = guard.clone()?;
        *applied_epoch = epoch;
        Some(mesh)
    }

    pub fn remounted_blake3(&self) -> Option<String> {
        self.remounted_blake3.lock().ok().and_then(|g| g.clone())
    }

    /// Early-out hint before expensive GET. Authoritative apply is
    /// [`Self::try_apply_world_remount`] (mesh + remounted/status under pending lock).
    pub fn remount_apply_allowed(&self, job_generation: u64) -> bool {
        self.remount_generation.load(Ordering::SeqCst) == job_generation
            && self.wire_cap().is_some()
    }

    fn bump_remount_generation(&self) {
        self.remount_generation.fetch_add(1, Ordering::SeqCst);
    }

    /// Invalidate in-flight auth-http / join / host workers (Logout, XR leave).
    pub fn bump_auth_work_epoch(&self) -> u64 {
        self.auth_work_epoch.fetch_add(1, Ordering::SeqCst) + 1
    }

    pub fn auth_work_epoch(&self) -> u64 {
        self.auth_work_epoch.load(Ordering::SeqCst)
    }

    pub fn auth_work_is_current(&self, epoch: u64) -> bool {
        self.auth_work_epoch() == epoch
    }

    pub fn local_user_root(&self) -> u32 {
        self.local_user_root.load(Ordering::Relaxed)
    }

    pub fn set_local_user_root(&self, entity_id: u32) {
        self.local_user_root.store(entity_id, Ordering::Relaxed);
        // Anchor changed — drop Redraw-frame eye cache until next begin_display_frame.
        if let Ok(mut slot) = self.eye_pose_cache.lock() {
            *slot = None;
        }
    }

    pub fn is_grabbable(&self, id: u32) -> bool {
        self.box_defs_snapshot().grabbable.contains(&id)
    }

    /// Cheap Arc clone of Manifest box defs (draw / Grab hot paths).
    pub fn box_defs_snapshot(&self) -> Arc<BoxDefs> {
        self.box_defs
            .lock()
            .ok()
            .map(|d| Arc::clone(&d))
            .unwrap_or_else(|| Arc::new(BoxDefs::default()))
    }

    /// Freeze display faces/α/pose cache for this client frame (once; before Grab/eye).
    ///
    /// Also refreshes [`Self::user_root_pose`] cache so Desktop intent/highlight/render
    /// and XR floor share one lookup for the frame.
    pub fn begin_display_frame(&self) {
        self.epoch.begin_display_frame(std::time::Instant::now());
        let id = self.local_user_root();
        let eye = self.compute_user_root_pose_for(id);
        if let Ok(mut slot) = self.eye_pose_cache.lock() {
            // Presence may have raced during compute — publish only if id still matches.
            if self.local_user_root() == id {
                *slot = Some((id, eye));
            } else {
                *slot = None;
            }
        }
    }

    /// Display TRS from the **frozen** display frame (same α / faces as GPU; x0091).
    ///
    /// Color channels stay from `wire`. Missing cache row → `wire` unchanged.
    /// Call [`Self::begin_display_frame`] (or ensure via sync) before Grab/draw/eye.
    /// Owned entities (x00ab) prefer prediction override when present.
    pub fn display_pose_for(&self, entity_id: u32, wire: EntityPose) -> EntityPose {
        self.display_pose_for_draw(entity_id, wire).0
    }

    /// Like [`Self::display_pose_for`], plus whether the pose is a local-prediction
    /// override (draw must use `model_override` in the same decision).
    ///
    /// Priority: owned Grab prediction (x00ab) → PendingLocalOps (x00e6) → Epoch display.
    pub fn display_pose_for_draw(&self, entity_id: u32, wire: EntityPose) -> (EntityPose, bool) {
        if let Ok(owned) = self.owned_prediction.lock() {
            if let Some(p) = owned.display_override(entity_id) {
                return (
                    EntityPose {
                        x: p.x,
                        y: p.y,
                        z: p.z,
                        qx: p.qx,
                        qy: p.qy,
                        qz: p.qz,
                        qw: p.qw,
                        sx: wire.sx,
                        sy: wire.sy,
                        sz: wire.sz,
                        r: wire.r,
                        g: wire.g,
                        b: wire.b,
                        a: wire.a,
                    },
                    true,
                );
            }
        }
        if self.pending_local.any_pending() {
            if let Some(p) = self.pending_local.display_override(entity_id) {
                return (
                    EntityPose {
                        x: p.x,
                        y: p.y,
                        z: p.z,
                        qx: p.qx,
                        qy: p.qy,
                        qz: p.qz,
                        qw: p.qw,
                        sx: wire.sx,
                        sy: wire.sy,
                        sz: wire.sz,
                        r: wire.r,
                        g: wire.g,
                        b: wire.b,
                        a: wire.a,
                    },
                    true,
                );
            }
        }
        let pose = self
            .epoch
            .with_display_frame(std::time::Instant::now(), |frame| {
                frame
                    .poses
                    .get(&entity_id)
                    .map(|out| entity_pose_from_gpu_row(out, wire))
                    .unwrap_or(wire)
            })
            .unwrap_or(wire);
        (pose, false)
    }

    /// Begin local Grab hold in the prediction tracker (Desktop Grab start).
    /// `false` if pending reject — Desktop must not set `held_grab` / Intent GRAB.
    ///
    /// x00f1: also mirrors into [`Self::pending_local`] (Published 非汚染の一般器).
    pub fn begin_held_grab(
        &self,
        entity_id: u32,
        pose: crate::owned_prediction::PredictionPose,
    ) -> bool {
        let Ok(mut owned) = self.owned_prediction.lock() else {
            return false;
        };
        if !owned.begin_held(entity_id, pose) {
            return false;
        }
        drop(owned);
        self.pending_local.push_pose(entity_id, pose);
        true
    }

    /// Drop prediction hold immediately (button release / dash). Keeps pending reject latch.
    pub fn clear_held_grab_prediction(&self) {
        let cleared_id = self.owned_prediction.lock().ok().and_then(|mut owned| {
            let id = owned.held_entity;
            owned.set_held(None);
            id
        });
        if let Some(id) = cleared_id {
            self.pending_local.clear(id);
        }
    }

    /// Push local Grab prediction only (every input tick). Does **not** reconcile —
    /// authority Snap/Keep runs on Applied Frame via [`Self::reconcile_held_on_authority`].
    ///
    /// Lock order: `owned_prediction` only (never acquire `world` under this lock).
    /// While `grab_rejected` is pending, updates are ignored (no hold resurrection).
    /// x00f1: mirrors pose into [`Self::pending_local`].
    pub fn predict_held_grab(
        &self,
        held_entity: Option<u32>,
        predicted_held: Option<crate::owned_prediction::PredictionPose>,
    ) {
        let Ok(mut owned) = self.owned_prediction.lock() else {
            return;
        };
        if owned.has_pending_reject() {
            return;
        }
        match (held_entity, predicted_held) {
            (Some(id), Some(pose)) => {
                // Only mirror when owned actually accepted the pose (x00f1 review).
                if owned.predict_held(id, pose) {
                    drop(owned);
                    self.pending_local.push_pose(id, pose);
                }
            }
            (None, _) => {
                let prev = owned.held_entity;
                owned.set_held(None);
                drop(owned);
                if let Some(id) = prev {
                    self.pending_local.clear(id);
                }
            }
            (Some(_), None) => {}
        }
    }

    /// Reconcile held Grab against one kinematic authority sample (Applied Frame).
    ///
    /// Missing `held_id` in `authority` → force reject (despawn / gap).
    /// x00f1: keeps [`Self::pending_local`] in sync (Snap/reject clears; Keep/Blend updates).
    pub fn reconcile_held_on_authority(
        &self,
        authority: &std::collections::HashMap<u32, crate::epoch_mirror::KinematicPose>,
    ) {
        let Ok(mut owned) = self.owned_prediction.lock() else {
            return;
        };
        let Some(id) = owned.held_entity else {
            return;
        };
        let Some(p) = authority.get(&id).copied() else {
            owned.force_reject();
            drop(owned);
            self.pending_local.clear(id);
            return;
        };
        let auth = crate::owned_prediction::PredictionPose::xyz_quat(
            p.x, p.y, p.z, p.qx, p.qy, p.qz, p.qw,
        );
        let result = owned.reconcile_held(auth);
        drop(owned);
        match result {
            Some((_, crate::owned_prediction::ReconcileMode::Snap)) => {
                self.pending_local.clear(id);
            }
            Some((pose, _)) => {
                self.pending_local.push_pose(id, pose);
            }
            None => {}
        }
    }

    /// Wire Curr for a specific `user_root` id (unset → default eye). Not for draw/Grab/view.
    fn user_root_wire_pose_for(&self, id: u32) -> EntityPose {
        if id == LOCAL_USER_ROOT_UNSET {
            return default_eye_pose();
        }
        self.world
            .lock()
            .ok()
            .and_then(|w| w.entities.get(&id).copied())
            .unwrap_or_else(default_eye_pose)
    }

    /// FPS / loco / XR-floor **eye** TRS from the frozen display snap (x0092).
    ///
    /// Viewpoint anchor remains `local_user_root` (FPS). TPS / Live camera are out of scope.
    /// Prefer after [`Self::begin_display_frame`] (fills cache). Mid-frame callers reuse that
    /// cache when the cached id still matches, else recompute.
    pub fn user_root_pose(&self) -> EntityPose {
        let id = self.local_user_root();
        if let Ok(slot) = self.eye_pose_cache.lock() {
            if let Some(pose) = eye_cache_hit(*slot, id) {
                return pose;
            }
        }
        self.compute_user_root_pose_for(id)
    }

    fn compute_user_root_pose_for(&self, id: u32) -> EntityPose {
        let wire = self.user_root_wire_pose_for(id);
        self.epoch
            .with_display_frame(std::time::Instant::now(), |frame| {
                resolve_user_root_display_pose(id, wire, Some(frame))
            })
            .unwrap_or_else(|| resolve_user_root_display_pose(id, wire, None))
    }

    /// Grabbable boxes with display TRS (hit aligned to draw; owned Grab uses prediction).
    pub fn grabbable_display_poses(&self) -> (Vec<(u32, EntityPose)>, Arc<BoxDefs>) {
        let defs = self.box_defs_snapshot();
        // Snapshot wires under `world`, then resolve display without holding `world`.
        let wires: Vec<(u32, EntityPose)> = self
            .world
            .lock()
            .ok()
            .map(|w| {
                w.entities
                    .iter()
                    .filter(|(id, _)| defs.grabbable.contains(*id))
                    .map(|(&id, &wire)| (id, wire))
                    .collect()
            })
            .unwrap_or_default();
        let boxes = wires
            .into_iter()
            .map(|(id, wire)| (id, self.display_pose_for(id, wire)))
            .collect();
        (boxes, defs)
    }

    pub fn mesh_entity_id(&self) -> u32 {
        self.mesh_entity_id.load(Ordering::Relaxed)
    }

    pub fn apply_manifest_defs(
        &self,
        grabbable: HashSet<u32>,
        half_extents: HashMap<u32, f32>,
        mesh_id: u32,
    ) {
        if let Ok(mut slot) = self.box_defs.lock() {
            *slot = Arc::new(BoxDefs {
                grabbable,
                half_extents,
            });
        }
        self.mesh_entity_id.store(mesh_id, Ordering::Relaxed);
    }

    /// Replace scene mesh list used by Inventory put-back (x006e).
    pub fn set_scene_meshes(&self, meshes: Vec<(u32, String)>) {
        if let Ok(mut slot) = self.scene_meshes.lock() {
            *slot = meshes;
        }
    }

    /// Snapshot of Manifest mesh entries `(entity_id, blake3)`.
    pub fn scene_meshes(&self) -> Vec<(u32, String)> {
        self.scene_meshes
            .lock()
            .map(|g| g.clone())
            .unwrap_or_default()
    }

    /// Host Frame / Presence / Manifest accepted — keepalive for Live stall UX (x007c).
    pub fn note_host_alive(&self) {
        if let Ok(mut slot) = self.last_host_alive_at.lock() {
            *slot = Some(Instant::now());
        }
    }

    /// Instant of last [`Self::note_host_alive`], if any.
    pub fn last_host_alive_at(&self) -> Option<Instant> {
        self.last_host_alive_at.lock().ok().and_then(|g| *g)
    }

    /// Desktop → XR: publish Drive rows + inventory status (x007f).
    ///
    /// `put_back_ok`: `Some` only for put_back outcomes so XR can wait on revision + flag
    /// without parsing status text (x00ce). `put_back_correlation` echoes the peer token.
    /// Returns `false` when the mirror lock is poisoned (caller may keep correlation).
    pub fn publish_inventory_mirror(
        &self,
        nodes: Vec<(String, String, String)>,
        status: String,
        put_back_ok: Option<bool>,
        put_back_correlation: Option<u64>,
    ) -> bool {
        let Ok(mut m) = self.inventory_mirror.lock() else {
            log::warn!("inventory_mirror lock poisoned — XR list not updated");
            return false;
        };
        m.nodes = nodes;
        m.status = status;
        m.last_put_back_ok = put_back_ok;
        m.last_put_back_correlation = if put_back_ok.is_some() {
            put_back_correlation
        } else {
            None
        };
        m.revision = m.revision.saturating_add(1);
        true
    }

    /// Current Zenoh `wire_cap` after join (x004e).
    pub fn wire_cap(&self) -> Option<Arc<str>> {
        self.wire_cap.lock().ok().and_then(|g| g.clone())
    }

    /// Current Room AEAD key after join (x0069).
    pub fn room_key(&self) -> Option<Arc<[u8; alchemy_protocol::wire_aead::ROOM_KEY_LEN]>> {
        self.room_key.lock().ok().and_then(|g| g.clone())
    }

    /// Current Room key epoch after join (x007e).
    pub fn room_key_epoch(&self) -> u64 {
        self.room_key_epoch.load(Ordering::SeqCst)
    }

    pub fn set_room_key_epoch(&self, epoch: u64) {
        self.room_key_epoch.store(epoch, Ordering::SeqCst);
    }

    pub fn key_stale(&self) -> bool {
        self.key_stale.load(Ordering::SeqCst)
    }

    pub fn set_key_stale(&self, stale: bool) {
        self.key_stale.store(stale, Ordering::SeqCst);
    }

    /// Token **refresh** path (x00d5): `live+1` stashes pending AEAD for dual-open;
    /// hard KeyStale when already stale, grace later fails, or epoch gap `> 1`.
    ///
    /// Includes `0 → 1`. Join/Rejoin use [`Self::commit_room_aead`] for first install.
    pub fn note_room_aead_from_refresh(
        &self,
        room_key: [u8; alchemy_protocol::wire_aead::ROOM_KEY_LEN],
        epoch: u64,
    ) -> bool {
        let prev = self.room_key_epoch();
        match refresh_aead_action(prev, epoch, self.key_stale()) {
            RefreshAeadAction::Commit => {
                self.commit_room_aead(room_key, epoch);
                false
            }
            RefreshAeadAction::PreparePending => {
                self.stash_pending_room_aead(room_key, epoch);
                self.schedule_room_key_epoch_probe(ROOM_KEY_EPOCH_PROBE);
                false
            }
            RefreshAeadAction::KeyStale => {
                if !self.key_stale() {
                    self.enter_key_stale(prev, epoch);
                }
                let _ = room_key;
                self.schedule_room_key_epoch_probe(ROOM_KEY_EPOCH_PROBE);
                true
            }
            RefreshAeadAction::IgnoreBackward => {
                log::warn!(
                    "room_key_epoch Token went backwards ({prev} → {epoch}); keeping live AEAD"
                );
                let _ = room_key;
                false
            }
            RefreshAeadAction::IgnoreWhileStale => {
                let _ = room_key;
                self.schedule_room_key_epoch_probe(ROOM_KEY_EPOCH_PROBE);
                true
            }
        }
    }

    /// Stash next key for seamless dual-open (x00d5). Live AEAD unchanged.
    fn stash_pending_room_aead(
        &self,
        room_key: [u8; alchemy_protocol::wire_aead::ROOM_KEY_LEN],
        epoch: u64,
    ) {
        let key = Arc::new(room_key);
        let Ok(mut slot) = self.pending_room_aead.lock() else {
            log::warn!("stash_pending_room_aead: mutex poisoned");
            return;
        };
        let renew = slot.as_ref().map(|p| p.epoch != epoch).unwrap_or(true);
        if renew {
            let deadline = Instant::now() + PENDING_ROOM_AEAD_GRACE;
            log::info!(
                "room_key_epoch advanced {} → {epoch} (pending AEAD; dual-open until Manifest)",
                self.room_key_epoch()
            );
            *slot = Some(PendingRoomAead {
                key,
                epoch,
                deadline,
            });
        } else if let Some(pending) = slot.as_mut() {
            // Same pending epoch — refresh key Arc + extend grace.
            pending.key = key;
            pending.deadline = Instant::now() + PENDING_ROOM_AEAD_GRACE;
        }
    }

    /// Join / Rejoin / first install / same-epoch soft-TTL sync: set live `(key, epoch)`.
    ///
    /// Clears pending only when it is not ahead of `epoch` (a delayed same-epoch `Commit`
    /// must not wipe a newer seamless stash · x00d5).
    /// `bus_generation` bumps only when the live epoch changes (not same-epoch soft-TTL).
    pub fn commit_room_aead(
        &self,
        room_key: [u8; alchemy_protocol::wire_aead::ROOM_KEY_LEN],
        epoch: u64,
    ) {
        let Ok(mut key_guard) = self.room_key.lock() else {
            log::warn!("commit_room_aead: room_key Mutex poisoned; refusing epoch-only update");
            return;
        };
        let prev_epoch = self.room_key_epoch();
        *key_guard = Some(Arc::new(room_key));
        self.set_room_key_epoch(epoch);
        if let Ok(mut pending) = self.pending_room_aead.lock() {
            clear_pending_unless_newer(&mut pending, epoch);
        }
        drop(key_guard);
        self.set_key_stale(false);
        if prev_epoch != epoch {
            self.bump_bus_generation();
        }
        self.schedule_room_key_epoch_probe(ROOM_KEY_EPOCH_PROBE);
    }

    /// Manifest Accept opened with pending epoch → live←pending under
    /// `room_key` → `pending` lock order (matches [`Self::leave_room_local`]; x00d5).
    ///
    /// Pending is cleared only after live is updated. Refuse if Leave already cleared
    /// live key or the room bus is down.
    pub fn try_commit_pending_room_aead(&self, opened_epoch: u64) -> bool {
        let bus_started = self.room_bus_started.load(Ordering::SeqCst);
        let Ok(mut key_guard) = self.room_key.lock() else {
            log::warn!("try_commit_pending_room_aead: room_key poisoned; pending kept");
            return false;
        };
        let Ok(mut pending_guard) = self.pending_room_aead.lock() else {
            return false;
        };
        let Some((_key, epoch)) = commit_pending_into_live(
            &mut key_guard,
            &mut pending_guard,
            opened_epoch,
            bus_started,
        ) else {
            return false;
        };
        self.set_room_key_epoch(epoch);
        drop(pending_guard);
        drop(key_guard);
        self.set_key_stale(false);
        self.bump_bus_generation();
        self.schedule_room_key_epoch_probe(ROOM_KEY_EPOCH_PROBE);
        log::info!("pending room AEAD committed epoch={epoch} (seamless; bus_generation bumped)");
        true
    }

    /// Grace deadline exceeded or hard fail → KeyStale overlay (x00d5).
    pub fn enter_key_stale(&self, prev_epoch: u64, token_epoch: u64) {
        if let Ok(mut pending) = self.pending_room_aead.lock() {
            *pending = None;
        }
        self.set_key_stale(true);
        self.bump_remount_generation();
        self.bump_bus_generation();
        log::info!("room_key_epoch {prev_epoch} → {token_epoch} (KeyStale; Rejoin required)");
    }

    /// Poll pending grace; returns true when hard KeyStale was entered this call.
    ///
    /// Deadline check and pending clear run under the **same** pending lock so a concurrent
    /// [`Self::try_commit_pending_room_aead`] cannot commit then be overwritten by KeyStale.
    pub fn poll_pending_room_aead_grace(&self) -> bool {
        if self.key_stale() {
            return false;
        }
        let Ok(mut slot) = self.pending_room_aead.lock() else {
            return false;
        };
        let prev = self.room_key_epoch();
        let Some(epoch) = take_pending_if_grace_expired(&mut slot, Instant::now()) else {
            return false;
        };
        drop(slot);
        self.set_key_stale(true);
        self.bump_remount_generation();
        self.bump_bus_generation();
        log::info!("room_key_epoch {prev} → {epoch} (KeyStale; pending grace expired)");
        true
    }

    pub fn bump_bus_generation(&self) {
        self.bus_generation.fetch_add(1, Ordering::SeqCst);
    }

    pub fn bus_generation(&self) -> u64 {
        self.bus_generation.load(Ordering::SeqCst)
    }

    /// Pending epoch while seamless switch is in flight (`None` if idle).
    pub fn pending_room_aead_epoch(&self) -> Option<u64> {
        self.pending_room_aead
            .lock()
            .ok()
            .and_then(|g| g.as_ref().map(|p| p.epoch))
    }

    /// Snapshot live `(room_key, epoch)` under one `room_key` lock (x007e wire boundary).
    pub fn live_room_aead(
        &self,
    ) -> Option<(Arc<[u8; alchemy_protocol::wire_aead::ROOM_KEY_LEN]>, u64)> {
        let Ok(guard) = self.room_key.lock() else {
            return None;
        };
        let key = guard.as_ref()?.clone();
        // Writers of the pair hold this mutex when updating epoch (`commit_room_aead`).
        let epoch = self.room_key_epoch.load(Ordering::SeqCst);
        Some((key, epoch))
    }

    /// Dual-open view: pending as current (if any), live as prev (x00d5).
    /// Returns `(current_key, current_epoch, prev, bus_generation)`.
    pub fn room_aead_open_view(&self) -> Option<RoomAeadOpenView> {
        let gen = self.bus_generation();
        let live = self.live_room_aead()?;
        let Ok(pending_guard) = self.pending_room_aead.lock() else {
            return Some((live.0, live.1, None, gen));
        };
        if let Some(pending) = pending_guard.as_ref() {
            let current_key = Arc::clone(&pending.key);
            let current_epoch = pending.epoch;
            let prev = Some(live);
            Some((current_key, current_epoch, prev, gen))
        } else {
            Some((live.0, live.1, None, gen))
        }
    }

    /// Seal key: pending if set, else live (x00d5). Returns `(key, epoch, bus_generation)`.
    pub fn room_aead_seal_view(
        &self,
    ) -> Option<(
        Arc<[u8; alchemy_protocol::wire_aead::ROOM_KEY_LEN]>,
        u64,
        u64,
    )> {
        let gen = self.bus_generation();
        let Ok(pending_guard) = self.pending_room_aead.lock() else {
            let (key, epoch) = self.live_room_aead()?;
            return Some((key, epoch, gen));
        };
        if let Some(pending) = pending_guard.as_ref() {
            Some((Arc::clone(&pending.key), pending.epoch, gen))
        } else {
            drop(pending_guard);
            let (key, epoch) = self.live_room_aead()?;
            Some((key, epoch, gen))
        }
    }

    /// Schedule the next guest epoch probe (Token re-fetch while joined).
    pub fn schedule_room_key_epoch_probe(&self, after: Duration) {
        if let Ok(mut slot) = self.room_key_epoch_probe_at.lock() {
            *slot = Some(Instant::now() + after);
        }
    }

    pub fn room_key_epoch_probe_due(&self, now: Instant) -> bool {
        self.room_key_epoch_probe_at
            .lock()
            .ok()
            .and_then(|g| *g)
            .is_some_and(|t| now >= t)
    }

    /// AEAD open failed while joined — nudge Token refresh so KeyStale can fire before FRAME_STALL.
    ///
    /// Not epoch-tagged: corrupt / kind-mismatch envelopes also refresh at most every 2s.
    /// Acceptable world load under rare wire errors; steady abuse still hits KeyStale or Leave.
    pub fn nudge_room_token_refresh_after_aead_fail(&self) {
        if self.key_stale() || !self.auth.lock().ok().is_some_and(|g| g.is_joined()) {
            return;
        }
        let Ok(mut slot) = self.aead_key_refresh_nudge_at.lock() else {
            return;
        };
        let now = Instant::now();
        if slot.is_some_and(|t| now.duration_since(t) < Duration::from_secs(2)) {
            return;
        }
        *slot = Some(now);
        drop(slot);
        self.force_room_token_refresh_soon();
    }

    pub fn leave_room_local(&self) {
        self.set_local_user_root(LOCAL_USER_ROOT_UNSET);
        if let Ok(mut intent) = self.intent.lock() {
            *intent = MoveIntent {
                suppress: intent.suppress,
                ..Default::default()
            };
        }
        if let Ok(mut cap) = self.wire_cap.lock() {
            *cap = None;
        }
        // Lock order: room_key → pending (matches try_commit_pending_room_aead · x00d5).
        // Prefer nested clear; if room_key is poisoned still drop pending so stash cannot
        // outlive Leave.
        match self.room_key.lock() {
            Ok(mut key) => {
                *key = None;
                // Pair with key clear under the same lock (`live_room_aead` / commit).
                self.set_room_key_epoch(0);
                if let Ok(mut pending) = self.pending_room_aead.lock() {
                    *pending = None;
                }
            }
            Err(_) => {
                log::error!(
                    "leave_room_local: room_key Mutex poisoned; clearing pending without key lock"
                );
                self.set_room_key_epoch(0);
                if let Ok(mut pending) = self.pending_room_aead.lock() {
                    *pending = None;
                }
            }
        }
        if let Ok(mut probe) = self.room_key_epoch_probe_at.lock() {
            *probe = None;
        }
        if let Ok(mut nudge) = self.aead_key_refresh_nudge_at.lock() {
            *nudge = None;
        }
        self.set_key_stale(false);
        self.bump_bus_generation();
        // Callers must `session.close()` (see `leave_room` / `close_zenoh`); Drop alone is not
        // enough if any strong clone remains. This flag only gates re-attach on the next open/bind.
        self.room_bus_started
            .store(false, std::sync::atomic::Ordering::SeqCst);
        self.mtls_reopen_pending
            .store(false, std::sync::atomic::Ordering::SeqCst);
        // Lock order: auth → intent_session → pending (matches Token commit paths).
        if let Ok(mut auth) = self.auth.lock() {
            auth.clear_room();
            if let Ok(mut session) = self.intent_session.lock() {
                *session = None;
                if let Ok(mut pending) = self.pending_intent_session.lock() {
                    *pending = None;
                }
            }
            if let Ok(mut retry) = self.pending_retry.lock() {
                *retry = None;
            }
        }
        // Same lock order as Manifest apply (host_pub → manifest_gen_rev) so leave
        // cannot interleave with AcceptSameOrForward while watermark still set (x003a).
        match (self.host_pub.lock(), self.manifest_gen_rev.lock()) {
            (Ok(mut host_pub), Ok(mut gen_rev)) => {
                *host_pub = None;
                *gen_rev = None;
            }
            _ => log::error!("leave_room_local: host_pub/manifest_gen_rev lock poisoned"),
        }
        if let Ok(mut edit_rev) = self.last_edit_scene_rev.lock() {
            *edit_rev = None;
        }
        if let Ok(mut pending) = self.pending_save_op_id.lock() {
            *pending = None;
        }
        if let Ok(mut save_st) = self.room_save_status.lock() {
            *save_st = None;
        }
        if let Ok(mut seal) = self.last_world_seal.lock() {
            *seal = None;
        }
        if let Ok(mut slot) = self.presence_gen_rev.lock() {
            *slot = None;
        }
        if let Ok(mut peers) = self.presence_peer_subs.lock() {
            peers.clear();
        }
        if let Ok(mut world) = self.world.lock() {
            *world = WorldView::default();
        }
        if let Ok(mut alive) = self.last_host_alive_at.lock() {
            *alive = None;
        }
        if let Ok(mut slot) = self.box_defs.lock() {
            *slot = Arc::new(BoxDefs::default());
        }
        self.mesh_entity_id.store(0, Ordering::Relaxed);
        self.set_scene_meshes(Vec::new());
        // GPU unload is surface-owned; bump + clear remounted/status + empty shell under one lock (x007d).
        self.finish_leave_empty_shell();
        if let Ok(mut token) = self.liveliness.lock() {
            *token = None;
        }
        if let Ok(mut prev) = self.liveliness_prev.lock() {
            *prev = None;
        }
    }

    /// Host generation change: clear world + Presence watermark (x0023 / x003a).
    /// Also clears `last_world_seal` (same lifetime as `leave_room_local`) so a mid-reset
    /// failure cannot keep a prior generation's seal for PlaceRead (x006c).
    /// x003e: mint new Intent key into `pending_intent_session` only; live publisher keeps
    /// old key + old Token until Room Token for the new pub arrives, then **atomic** swap.
    ///
    /// GPU world mesh is intentionally left until the next remount GET completes (no empty-shell
    /// flash mid-session). In-flight remount jobs (mesh **and** `remounted_blake3` write-back)
    /// are discarded via [`Self::invalidate_remount_for_host_generation`].
    pub fn reset_for_host_generation(&self) {
        if let Ok(mut world) = self.world.lock() {
            *world = WorldView::default();
        }
        if let Ok(mut alive) = self.last_host_alive_at.lock() {
            *alive = None;
        }
        if let Ok(mut slot) = self.box_defs.lock() {
            *slot = Arc::new(BoxDefs::default());
        }
        self.mesh_entity_id.store(0, Ordering::Relaxed);
        self.set_scene_meshes(Vec::new());
        self.invalidate_remount_for_host_generation();
        if let Ok(mut seal) = self.last_world_seal.lock() {
            *seal = None;
        }
        if let Ok(mut slot) = self.presence_gen_rev.lock() {
            *slot = None;
        }

        let Some((session_pub_b64, access)) = self.try_install_pending_for_generation() else {
            return;
        };

        let Some(access) = access else {
            log::warn!(
                "host generation: joined with live session but no access_token; pending kept, schedule retry"
            );
            self.note_pending_token_failure();
            return;
        };

        let (csr_pem, eph_key) = if zenoh_connect::should_request_world_mtls() {
            match zenoh_connect::EphemeralMtlsIdentity::generate() {
                Ok(eph) => (
                    Some(eph.csr_pem.clone()),
                    Some(eph.private_key_pem().to_string()),
                ),
                Err(err) => {
                    log::warn!("host generation CSR failed: {err:#}");
                    (None, None)
                }
            }
        } else {
            (None, None)
        };
        let world_url = self.get_world_url();
        let room_id = self.get_room_id();
        let refresh_host_inventory = self
            .auth
            .lock()
            .ok()
            .is_some_and(|g| g.host_room_token.is_some());
        match crate::world_token::fetch_room_token(
            &world_url,
            room_id.as_ref(),
            &access,
            &session_pub_b64,
            csr_pem.as_deref(),
            "guest",
        ) {
            Ok(guest) => {
                match zenoh_connect::apply_world_mtls_or_allocate(
                    guest.mtls.as_ref().map(|m| m.client_cert_pem.as_str()),
                    eph_key.as_deref(),
                    &guest.wire_cap,
                    "guest",
                    &guest.allocation_token,
                    guest
                        .mtls
                        .as_ref()
                        .map(|m| m.expires_at.as_str())
                        .filter(|s| !s.is_empty()),
                ) {
                    Ok(Some(id)) => {
                        self.mtls_reopen_pending
                            .store(true, std::sync::atomic::Ordering::SeqCst);
                        log::info!(
                            "host generation mTLS cn={} soft_ttl={}s (mtls_reopen_pending)",
                            id.cn,
                            id.expires_in
                        );
                    }
                    Ok(None) => {}
                    Err(err) => log::warn!("host generation mTLS update failed: {err:#}"),
                }
                let local_sub = match crate::jwt_sub::sub_unverified(&guest.token) {
                    Ok(s) => s,
                    Err(err) => {
                        log::warn!(
                            "host generation Room Token: bad sub: {err:#}; schedule refresh retry"
                        );
                        if self.has_pending_intent_session_joined() {
                            self.note_pending_token_failure();
                        }
                        return;
                    }
                };
                if self.commit_pending_session_with_token(&session_pub_b64, guest, local_sub) {
                    log::info!(
                        "Room Token + Intent session swapped for new host generation (cnf aligned)"
                    );
                    if refresh_host_inventory {
                        if let Some(access) =
                            self.auth.lock().ok().and_then(|g| g.access_token.clone())
                        {
                            if let Err(err) =
                                crate::room::install_host_inventory_token(self, &access)
                            {
                                log::warn!("host inventory Token after generation failed: {err:#}");
                                if let Ok(mut gate) = self.auth.lock() {
                                    gate.set_host_room_token(None);
                                }
                            }
                        }
                    }
                } else {
                    log::warn!(
                        "host generation commit skipped (leave/newer pending); schedule retry if still joined"
                    );
                    if self.has_pending_intent_session_joined() {
                        self.note_pending_token_failure();
                    }
                }
            }
            Err(err) => {
                log::warn!(
                    "host generation Room Token sync fetch failed: {err:#}; keep old key, retry via refresh"
                );
                if self.has_pending_intent_session_joined() {
                    self.note_pending_token_failure();
                }
            }
        }
    }

    /// Install pending Intent key only while joined with a live session.
    /// Lock order: auth → intent_session → pending → pending_retry.
    /// Returns `(session_pub_b64, access_token)` or `None` if not eligible / lock failure.
    fn try_install_pending_for_generation(&self) -> Option<(String, Option<String>)> {
        let Ok(auth) = self.auth.lock() else {
            log::warn!("host generation: auth lock failed; skip pending install");
            return None;
        };
        let Ok(sess) = self.intent_session.lock() else {
            log::warn!("host generation: intent_session lock failed; skip pending install");
            return None;
        };
        let Ok(mut pending) = self.pending_intent_session.lock() else {
            log::warn!("host generation: pending lock failed; skip pending install");
            return None;
        };
        if !auth.is_joined() || sess.is_none() {
            return None;
        }
        let new_session = crate::intent_session::IntentSession::new();
        let session_pub_b64 = new_session.session_pub_b64();
        *pending = Some(new_session);
        if let Ok(mut retry) = self.pending_retry.lock() {
            *retry = None;
        }
        let access = auth.access_token.clone();
        // Defer epoch probe: pending suppresses fetch; a past-due probe_at must not pin
        // token_refresh_wait after pending clears. Drop lock order before probe mutex.
        drop(pending);
        drop(sess);
        drop(auth);
        self.schedule_room_key_epoch_probe(ROOM_KEY_EPOCH_PROBE);
        log::info!("pending Intent session for host generation (live key unchanged until Token)");
        Some((session_pub_b64, access))
    }

    /// True when a host-generation Intent key awaits matching Room Token **and** still joined.
    /// Lock order: auth → intent_session → pending.
    pub fn has_pending_intent_session_joined(&self) -> bool {
        let Ok(auth) = self.auth.lock() else {
            return false;
        };
        if !auth.is_joined() {
            return false;
        }
        let Ok(_sess) = self.intent_session.lock() else {
            return false;
        };
        self.pending_intent_session
            .lock()
            .ok()
            .map(|p| p.is_some())
            .unwrap_or(false)
    }

    /// Pending session_pub while joined, else `None`. Lock order: auth → intent → pending.
    pub fn pending_session_pub_b64_if_joined(&self) -> Option<String> {
        let Ok(auth) = self.auth.lock() else {
            return None;
        };
        if !auth.is_joined() {
            return None;
        }
        let Ok(_sess) = self.intent_session.lock() else {
            return None;
        };
        let Ok(pending) = self.pending_intent_session.lock() else {
            return None;
        };
        pending.as_ref().map(|s| s.session_pub_b64())
    }

    /// Live session_pub while joined (no pending install). Lock order: auth → intent → pending.
    pub fn live_session_pub_b64_if_joined(&self) -> Option<String> {
        let Ok(auth) = self.auth.lock() else {
            return None;
        };
        if !auth.is_joined() {
            return None;
        }
        let Ok(sess) = self.intent_session.lock() else {
            return None;
        };
        let Ok(_pending) = self.pending_intent_session.lock() else {
            return None;
        };
        sess.as_ref().map(|s| s.session_pub_b64())
    }

    /// Drop orphan pending after leave (lock order: auth → intent → pending → retry).
    pub fn clear_pending_if_not_joined(&self) {
        let Ok(auth) = self.auth.lock() else {
            return;
        };
        if auth.is_joined() {
            return;
        }
        let Ok(_sess) = self.intent_session.lock() else {
            return;
        };
        let Ok(mut pending) = self.pending_intent_session.lock() else {
            return;
        };
        if pending.is_some() {
            log::info!("clear orphan pending Intent session (not joined)");
            *pending = None;
        }
        if let Ok(mut retry) = self.pending_retry.lock() {
            *retry = None;
        }
    }

    /// Back off pending Token retries (1s → 2s → … → 30s).
    pub fn note_pending_token_failure(&self) {
        const BACKOFF_START: Duration = Duration::from_secs(1);
        const BACKOFF_MAX: Duration = Duration::from_secs(30);
        let now = Instant::now();
        let Ok(mut retry) = self.pending_retry.lock() else {
            return;
        };
        let last_wait = retry.map(|(_, w)| w).unwrap_or(Duration::ZERO);
        let wait = if last_wait.is_zero() {
            BACKOFF_START
        } else {
            last_wait.saturating_mul(2).min(BACKOFF_MAX)
        };
        *retry = Some((now + wait, wait));
    }

    /// Clear pending fetch backoff after a successful commit.
    pub fn clear_pending_token_backoff(&self) {
        if let Ok(mut retry) = self.pending_retry.lock() {
            *retry = None;
        }
    }

    /// When to next attempt a pending Token fetch (`None` = immediately if pending).
    pub fn pending_retry_not_before(&self) -> Option<Instant> {
        self.pending_retry
            .lock()
            .ok()
            .and_then(|g| g.map(|(t, _)| t))
    }

    /// Commit pending Intent key + Room Token together.
    /// Lock order: auth → intent_session → pending_intent_session.
    pub fn commit_pending_session_with_token(
        &self,
        expected_pub_b64: &str,
        guest: crate::world_token::GuestToken,
        local_sub: String,
    ) -> bool {
        let Ok(mut auth) = self.auth.lock() else {
            return false;
        };
        let Ok(mut sess) = self.intent_session.lock() else {
            return false;
        };
        let Ok(mut pending) = self.pending_intent_session.lock() else {
            return false;
        };
        if !auth.is_joined() {
            return false;
        }
        let Some(pending_sess) = pending.as_ref() else {
            return false;
        };
        if pending_sess.session_pub_b64() != expected_pub_b64 {
            return false;
        }
        let Some(mut new_sess) = pending.take() else {
            return false;
        };
        new_sess.mark_bind_pending();
        let room_key = guest.room_key;
        let room_key_epoch = guest.room_key_epoch;
        auth.apply_room_refresh(guest.token, local_sub, guest.expires_in);
        *sess = Some(new_sess);
        drop(pending);
        drop(sess);
        drop(auth);
        self.note_room_aead_from_refresh(room_key, room_key_epoch);
        self.clear_pending_token_backoff();
        true
    }

    /// Refresh current live session Token only if `session_pub` still matches.
    /// Lock order: auth → intent_session → pending_intent_session.
    pub fn apply_room_token_if_session_matches(
        &self,
        expected_pub_b64: &str,
        guest: crate::world_token::GuestToken,
        local_sub: String,
    ) -> bool {
        let Ok(mut auth) = self.auth.lock() else {
            return false;
        };
        let Ok(mut sess) = self.intent_session.lock() else {
            return false;
        };
        let Ok(pending) = self.pending_intent_session.lock() else {
            return false;
        };
        if !auth.is_joined() {
            return false;
        }
        let Some(s) = sess.as_mut() else {
            return false;
        };
        if s.session_pub_b64() != expected_pub_b64 {
            return false;
        }
        let room_key = guest.room_key;
        let room_key_epoch = guest.room_key_epoch;
        auth.apply_room_refresh(guest.token, local_sub, guest.expires_in);
        s.mark_bind_pending();
        drop(pending);
        drop(sess);
        drop(auth);
        self.note_room_aead_from_refresh(room_key, room_key_epoch);
        true
    }

    /// Wake the background refresh loop without waiting a full poll interval.
    pub fn force_room_token_refresh_soon(&self) {
        if let Ok(mut auth) = self.auth.lock() {
            if auth.is_joined() {
                auth.room_deadline = Some(Instant::now());
            }
        }
    }

    /// Same host dropped our session (Presence reclaim): keep key, resend Room Token binds.
    /// Also re-declare liveliness — reclaim often follows wire Delete of the current jti.
    pub fn request_intent_rebind(&self) {
        let mut marked_bind = false;
        let mut already_pending = false;
        if let Ok(mut sess) = self.intent_session.lock() {
            if let Some(s) = sess.as_mut() {
                if s.bind_pending() {
                    already_pending = true;
                } else {
                    s.mark_bind_pending();
                    marked_bind = true;
                }
            }
        }
        let mut liveliness_set = false;
        if let Ok(mut auth) = self.auth.lock() {
            if auth.is_joined() {
                if !auth.liveliness_rebind {
                    liveliness_set = true;
                }
                auth.liveliness_rebind = true;
            }
        }
        if marked_bind || liveliness_set {
            log::info!(
                "intent+liveliness rebind pending (Presence missing while steady; bind_new={marked_bind} liveliness_new={liveliness_set})"
            );
        } else if already_pending {
            log::debug!(
                "intent+liveliness rebind reaffirmed (bind already pending; liveliness kept true)"
            );
        }
    }
}

/// Build world mesh GPU buffers. Empty shell uses a 1-vert dummy + `index_count=0` (x007d).
fn world_mesh_buffers(device: &wgpu::Device, mesh: &CpuMesh) -> (wgpu::Buffer, wgpu::Buffer, u32) {
    if mesh.indices.is_empty() {
        let dummy = [MeshVertex {
            position: [0.0, 0.0, 0.0],
        }];
        let vertex_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("mesh-vertices-empty"),
            contents: bytemuck::cast_slice(&dummy),
            usage: wgpu::BufferUsages::VERTEX,
        });
        let index_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("mesh-indices-empty"),
            contents: bytemuck::cast_slice(&[0u32]),
            usage: wgpu::BufferUsages::INDEX,
        });
        return (vertex_buffer, index_buffer, 0);
    }
    let vertices: Vec<MeshVertex> = mesh
        .positions
        .iter()
        .map(|p| MeshVertex { position: *p })
        .collect();
    let vertex_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
        label: Some("mesh-vertices"),
        contents: bytemuck::cast_slice(&vertices),
        usage: wgpu::BufferUsages::VERTEX,
    });
    let index_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
        label: Some("mesh-indices"),
        contents: bytemuck::cast_slice(&mesh.indices),
        usage: wgpu::BufferUsages::INDEX,
    });
    (vertex_buffer, index_buffer, mesh.indices.len() as u32)
}

pub struct SceneGpu {
    pub device: wgpu::Device,
    pub queue: wgpu::Queue,
    pub pipeline: wgpu::RenderPipeline,
    pub surface_format: wgpu::TextureFormat,
    vertex_buffer: wgpu::Buffer,
    index_buffer: wgpu::Buffer,
    index_count: u32,
    floor_vertex_buffer: wgpu::Buffer,
    floor_index_buffer: wgpu::Buffer,
    floor_index_count: u32,
    floor_uniform_buffer: wgpu::Buffer,
    floor_bind_group: wgpu::BindGroup,
    /// One UBO per mesh instance — same remounted GLB at each scene mesh pose.
    mesh_uniform_buffers: Vec<wgpu::Buffer>,
    mesh_bind_groups: Vec<wgpu::BindGroup>,
    box_vertex_buffer: wgpu::Buffer,
    box_index_buffer: wgpu::Buffer,
    box_index_count: u32,
    /// One uniform+bind group per box slot — wgpu cannot safely rewrite one UBO mid-pass.
    box_uniform_buffers: Vec<wgpu::Buffer>,
    box_bind_groups: Vec<wgpu::BindGroup>,
    mesh_radius: f32,
    shared: Arc<SharedState>,
    last_pose_log: Instant,
    /// Prev/Curr/Out storage + params for appearance-only interpolate (x0090/x0091).
    epoch_buffers: crate::epoch_view::EpochGpuBuffers,
    epoch_interp_pipeline: wgpu::ComputePipeline,
    epoch_interp_bg: wgpu::BindGroup,
    /// Render bind layout (uniform + epoch_out storage).
    draw_bgl: wgpu::BindGroupLayout,
    /// `entity_id` → index in last `epoch_out` pack (full active; x0091).
    epoch_entity_pack: HashMap<u32, u32>,
}

impl SceneGpu {
    pub fn from_device(
        device: wgpu::Device,
        queue: wgpu::Queue,
        surface_format: wgpu::TextureFormat,
        mesh: &CpuMesh,
        shared: Arc<SharedState>,
    ) -> Result<Self> {
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("mesh"),
            source: wgpu::ShaderSource::Wgsl(include_str!("shader.wgsl").into()),
        });

        let bind_group_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("uniform-epoch-bgl"),
            entries: &[
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::VERTEX | wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Uniform,
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 1,
                    visibility: wgpu::ShaderStages::VERTEX,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Storage { read_only: true },
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                },
            ],
        });

        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("pipeline-layout"),
            bind_group_layouts: &[&bind_group_layout],
            push_constant_ranges: &[],
        });

        let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("mesh-pipeline"),
            layout: Some(&pipeline_layout),
            vertex: wgpu::VertexState {
                module: &shader,
                entry_point: Some("vs_main"),
                buffers: &[wgpu::VertexBufferLayout {
                    array_stride: std::mem::size_of::<MeshVertex>() as wgpu::BufferAddress,
                    step_mode: wgpu::VertexStepMode::Vertex,
                    attributes: &wgpu::vertex_attr_array![0 => Float32x3],
                }],
                compilation_options: Default::default(),
            },
            fragment: Some(wgpu::FragmentState {
                module: &shader,
                entry_point: Some("fs_main"),
                targets: &[Some(wgpu::ColorTargetState {
                    format: surface_format,
                    blend: Some(wgpu::BlendState::REPLACE),
                    write_mask: wgpu::ColorWrites::ALL,
                })],
                compilation_options: Default::default(),
            }),
            primitive: wgpu::PrimitiveState {
                topology: wgpu::PrimitiveTopology::TriangleList,
                cull_mode: None,
                ..Default::default()
            },
            depth_stencil: Some(wgpu::DepthStencilState {
                format: wgpu::TextureFormat::Depth32Float,
                depth_write_enabled: true,
                depth_compare: wgpu::CompareFunction::Less,
                stencil: wgpu::StencilState::default(),
                bias: wgpu::DepthBiasState::default(),
            }),
            multisample: wgpu::MultisampleState::default(),
            multiview: None,
            cache: None,
        });

        let (vertex_buffer, index_buffer, index_count) = world_mesh_buffers(&device, mesh);

        let e = FLOOR_EXTENT;
        let floor_verts = [
            MeshVertex {
                position: [-e, 0.0, -e],
            },
            MeshVertex {
                position: [e, 0.0, -e],
            },
            MeshVertex {
                position: [e, 0.0, e],
            },
            MeshVertex {
                position: [-e, 0.0, e],
            },
        ];
        let floor_indices: [u32; 6] = [0, 1, 2, 0, 2, 3];
        let floor_vertex_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("floor-vertices"),
            contents: bytemuck::cast_slice(&floor_verts),
            usage: wgpu::BufferUsages::VERTEX,
        });
        let floor_index_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("floor-indices"),
            contents: bytemuck::cast_slice(&floor_indices),
            usage: wgpu::BufferUsages::INDEX,
        });

        let floor_uniform_buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("floor-uniforms"),
            size: std::mem::size_of::<Uniforms>() as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        const EPOCH_GPU_INITIAL_CAPACITY: u32 = 64;
        let (epoch_interp_pipeline, epoch_interp_bgl) =
            crate::epoch_view::create_interpolate_pipeline(&device);
        let epoch_buffers =
            crate::epoch_view::EpochGpuBuffers::new(&device, EPOCH_GPU_INITIAL_CAPACITY);
        let epoch_interp_bg =
            crate::epoch_view::make_interp_bind_group(&device, &epoch_interp_bgl, &epoch_buffers);

        let mut mesh_uniform_buffers = Vec::with_capacity(MESH_DRAW_SLOTS);
        let mut mesh_bind_groups = Vec::with_capacity(MESH_DRAW_SLOTS);
        for i in 0..MESH_DRAW_SLOTS {
            let buf = device.create_buffer(&wgpu::BufferDescriptor {
                label: Some(&format!("mesh-uniforms-{i}")),
                size: std::mem::size_of::<Uniforms>() as u64,
                usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
                mapped_at_creation: false,
            });
            let bg = make_draw_bind_group(
                &device,
                &bind_group_layout,
                &buf,
                &epoch_buffers.out,
                &format!("mesh-bg-{i}"),
            );
            mesh_uniform_buffers.push(buf);
            mesh_bind_groups.push(bg);
        }

        let (box_verts, box_indices) = unit_cube_mesh();
        let box_vertex_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("box-vertices"),
            contents: bytemuck::cast_slice(&box_verts),
            usage: wgpu::BufferUsages::VERTEX,
        });
        let box_index_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("box-indices"),
            contents: bytemuck::cast_slice(&box_indices),
            usage: wgpu::BufferUsages::INDEX,
        });
        let mut box_uniform_buffers = Vec::with_capacity(BOX_DRAW_SLOTS);
        let mut box_bind_groups = Vec::with_capacity(BOX_DRAW_SLOTS);
        for i in 0..BOX_DRAW_SLOTS {
            let buf = device.create_buffer(&wgpu::BufferDescriptor {
                label: Some(&format!("box-uniforms-{i}")),
                size: std::mem::size_of::<Uniforms>() as u64,
                usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
                mapped_at_creation: false,
            });
            let bg = make_draw_bind_group(
                &device,
                &bind_group_layout,
                &buf,
                &epoch_buffers.out,
                &format!("box-bg-{i}"),
            );
            box_uniform_buffers.push(buf);
            box_bind_groups.push(bg);
        }

        let floor_bind_group = make_draw_bind_group(
            &device,
            &bind_group_layout,
            &floor_uniform_buffer,
            &epoch_buffers.out,
            "floor-bg",
        );

        Ok(Self {
            device,
            queue,
            pipeline,
            surface_format,
            vertex_buffer,
            index_buffer,
            index_count,
            floor_vertex_buffer,
            floor_index_buffer,
            floor_index_count: floor_indices.len() as u32,
            floor_uniform_buffer,
            floor_bind_group,
            mesh_uniform_buffers,
            mesh_bind_groups,
            box_vertex_buffer,
            box_index_buffer,
            box_index_count: box_indices.len() as u32,
            box_uniform_buffers,
            box_bind_groups,
            mesh_radius: mesh.radius,
            shared,
            last_pose_log: Instant::now(),
            epoch_buffers,
            epoch_interp_pipeline,
            epoch_interp_bg,
            draw_bgl: bind_group_layout,
            epoch_entity_pack: HashMap::new(),
        })
    }

    /// Replace world mesh buffers (Manifest remount / Leave → empty shell; x007d).
    pub fn replace_world_mesh(&mut self, mesh: &CpuMesh) {
        let (vertex_buffer, index_buffer, index_count) = world_mesh_buffers(&self.device, mesh);
        self.vertex_buffer = vertex_buffer;
        self.index_buffer = index_buffer;
        self.index_count = index_count;
        self.mesh_radius = mesh.radius.max(1e-3);
        log::info!(
            "SceneGpu world mesh remounted: indices={index_count} radius={:.3}",
            self.mesh_radius
        );
    }

    pub fn coin_pose(&self) -> EntityPose {
        let mesh_id = self.shared.mesh_entity_id();
        let id = if mesh_id == 0 {
            MESH_ENTITY_ID
        } else {
            mesh_id
        };
        self.shared
            .world
            .lock()
            .ok()
            .and_then(|w| w.entities.get(&id).copied())
            .unwrap_or_default()
    }

    /// Display-frame eye at `local_user_root` (x0092; delegates to [`SharedState::user_root_pose`]).
    pub fn user_root_pose(&self) -> EntityPose {
        self.shared.user_root_pose()
    }

    pub fn look(&self) -> CameraLook {
        self.shared.look.lock().ok().map(|g| *g).unwrap_or_default()
    }

    pub fn maybe_log_pose(&mut self) {
        let coin = self.coin_pose();
        let user = self.user_root_pose();
        if self.last_pose_log.elapsed() >= Duration::from_secs(1) {
            log::info!(
                "poses: coin=({:.3},{:.3},{:.3}) user_root=({:.3},{:.3},{:.3})",
                coin.x,
                coin.y,
                coin.z,
                user.x,
                user.y,
                user.z
            );
            self.last_pose_log = Instant::now();
        }
    }

    /// Desktop FPS camera at user_root with client-local yaw/pitch.
    pub fn desktop_view_proj(eye: EntityPose, look: CameraLook, aspect: f32) -> (Mat4, Mat4) {
        let yaw = look.yaw;
        let pitch = look.pitch.clamp(-1.54, 1.54);
        let forward = Vec3::new(
            yaw.sin() * pitch.cos(),
            pitch.sin(),
            -yaw.cos() * pitch.cos(),
        )
        .normalize();
        let eye_pos = Vec3::new(eye.x, eye.y, eye.z);
        let view = Mat4::look_at_rh(eye_pos, eye_pos + forward, Vec3::Y);
        let proj = Mat4::perspective_rh(70.0_f32.to_radians(), aspect, 0.05, 200.0);
        (view, proj)
    }

    /// Upload Prev/Curr (full active), dispatch interpolate, refresh entity→pack map.
    ///
    /// Call **once per displayed frame** (Desktop once; XR once before the eye loop) —
    /// not inside per-eye [`Self::draw_world`]. Draw reads `epoch_out` (x0091).
    ///
    /// CPU pose cache (Grab/color) and GPU compute both apply the same frozen α —
    /// intentional dual path (no readback). Future MDI / CPU-only draw may drop compute.
    pub fn sync_epoch_gpu(&mut self, encoder: &mut wgpu::CommandEncoder) {
        // Reuse frame-start freeze (Grab/color); only begin here if nothing froze yet.
        let Some(frame) = self.shared.epoch.display_frame_snapshot() else {
            return;
        };
        let n = frame.curr_rows.len() as u32;
        if n > self.epoch_buffers.capacity {
            let new_cap = n.next_power_of_two().max(64);
            self.epoch_buffers = crate::epoch_view::EpochGpuBuffers::new(&self.device, new_cap);
            let layout = self.epoch_interp_pipeline.get_bind_group_layout(0);
            self.epoch_interp_bg = crate::epoch_view::make_interp_bind_group(
                &self.device,
                &layout,
                &self.epoch_buffers,
            );
            self.rebuild_draw_bind_groups();
        }
        crate::epoch_view::fill_entity_pack_index(
            &mut self.epoch_entity_pack,
            &frame.faces.curr,
            &frame.curr_rows,
        );
        self.epoch_buffers.upload_faces(
            &self.queue,
            &frame.prev_rows,
            &frame.curr_rows,
            frame.alpha,
        );
        if n == 0 {
            return;
        }
        {
            let mut cpass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
                label: Some("epoch-interpolate"),
                timestamp_writes: None,
            });
            cpass.set_pipeline(&self.epoch_interp_pipeline);
            cpass.set_bind_group(0, &self.epoch_interp_bg, &[]);
            cpass.dispatch_workgroups(n.div_ceil(64), 1, 1);
        }
    }

    fn rebuild_draw_bind_groups(&mut self) {
        self.floor_bind_group = make_draw_bind_group(
            &self.device,
            &self.draw_bgl,
            &self.floor_uniform_buffer,
            &self.epoch_buffers.out,
            "floor-bg",
        );
        for (i, buf) in self.mesh_uniform_buffers.iter().enumerate() {
            self.mesh_bind_groups[i] = make_draw_bind_group(
                &self.device,
                &self.draw_bgl,
                buf,
                &self.epoch_buffers.out,
                &format!("mesh-bg-{i}"),
            );
        }
        for (i, buf) in self.box_uniform_buffers.iter().enumerate() {
            self.box_bind_groups[i] = make_draw_bind_group(
                &self.device,
                &self.draw_bgl,
                buf,
                &self.epoch_buffers.out,
                &format!("box-bg-{i}"),
            );
        }
    }

    /// Resolve draw uniforms: epoch pack when mirrored, else CPU `EntityPose` model.
    #[allow(clippy::too_many_arguments)] // draw path packs epoch/owned flags with TRS
    fn draw_uniforms_for(
        &self,
        view_proj: Mat4,
        entity_id: u32,
        pose: EntityPose,
        pred_override: bool,
        scale_mul: Vec3,
        extra_rot: glam::Quat,
        color: [f32; 4],
    ) -> Uniforms {
        // Local prediction override (x00ab Grab or x00e6 PendingLocalOps) must win over
        // Epoch `out[pack]` (which ignores CPU TRS). Flag comes from `display_pose_for_draw`.
        if pred_override {
            let model = Mat4::from_scale_rotation_translation(
                pose.scale() * scale_mul,
                pose.rotation() * extra_rot,
                pose.position(),
            );
            return Uniforms::with_model(view_proj, model, color);
        }
        if let Some(&pack) = self.epoch_entity_pack.get(&entity_id) {
            Uniforms::with_epoch(view_proj, pack, scale_mul, extra_rot, color)
        } else {
            let model = Mat4::from_scale_rotation_translation(
                pose.scale() * scale_mul,
                pose.rotation() * extra_rot,
                pose.position(),
            );
            Uniforms::with_model(view_proj, model, color)
        }
    }

    #[allow(clippy::too_many_arguments)] // render pass bundling; highlight is Desktop-only pad
    pub fn draw_world(
        &self,
        encoder: &mut wgpu::CommandEncoder,
        color_view: &wgpu::TextureView,
        depth_view: &wgpu::TextureView,
        view: Mat4,
        proj: Mat4,
        load_op: wgpu::LoadOp<wgpu::Color>,
        // Desktop aim / held Grab highlight (entity id), if any.
        highlight_entity: Option<u32>,
    ) {
        {
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("scene-pass"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: color_view,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: load_op,
                        store: wgpu::StoreOp::Store,
                    },
                })],
                depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                    view: depth_view,
                    depth_ops: Some(wgpu::Operations {
                        load: wgpu::LoadOp::Clear(1.0),
                        store: wgpu::StoreOp::Store,
                    }),
                    stencil_ops: None,
                }),
                timestamp_writes: None,
                occlusion_query_set: None,
            });
            pass.set_pipeline(&self.pipeline);

            let view_proj = proj * view;

            // MainHub / Leave empty shell: clear color only (no floor / world mesh; x007d).
            if self.index_count > 0 {
                let floor_uniforms =
                    Uniforms::with_model(view_proj, Mat4::IDENTITY, [0.22, 0.24, 0.28, 1.0]);
                self.queue.write_buffer(
                    &self.floor_uniform_buffer,
                    0,
                    bytemuck::bytes_of(&floor_uniforms),
                );
                pass.set_bind_group(0, &self.floor_bind_group, &[]);
                pass.set_vertex_buffer(0, self.floor_vertex_buffer.slice(..));
                pass.set_index_buffer(self.floor_index_buffer.slice(..), wgpu::IndexFormat::Uint32);
                pass.draw_indexed(0..self.floor_index_count, 0, 0..1);

                // Instance the remounted GLB at every Manifest mesh pose (take-out siblings).
                // Different blake3 still share this GPU mesh until multi-GLB remount exists.
                let mesh_ids: Vec<u32> = {
                    let listed: Vec<u32> = self
                        .shared
                        .scene_meshes()
                        .into_iter()
                        .map(|(id, _)| id)
                        .collect();
                    if listed.is_empty() {
                        let mid = self.shared.mesh_entity_id();
                        if mid != 0 {
                            vec![mid]
                        } else {
                            vec![MESH_ENTITY_ID]
                        }
                    } else {
                        listed
                    }
                };
                let poses: Vec<(u32, EntityPose)> = self
                    .shared
                    .world
                    .lock()
                    .ok()
                    .map(|w| {
                        mesh_ids
                            .iter()
                            .filter_map(|id| w.entities.get(id).copied().map(|p| (*id, p)))
                            .collect()
                    })
                    .unwrap_or_default();
                let visual_scale = (alchemy_shared_sim::PHYS_BALL_RADIUS * 0.95
                    / self.mesh_radius.max(1e-3))
                .clamp(0.02, 4.0);
                let mesh_tilt = glam::Quat::from_rotation_x(0.25);
                pass.set_vertex_buffer(0, self.vertex_buffer.slice(..));
                pass.set_index_buffer(self.index_buffer.slice(..), wgpu::IndexFormat::Uint32);
                for (slot, (id, wire)) in poses.into_iter().enumerate().take(MESH_DRAW_SLOTS) {
                    // Mesh asset tilt stays client-local; TRS from display (Epoch or local pred).
                    // Height gradient uses display Y so color tracks the visible pose.
                    let (display, pred_override) = self.shared.display_pose_for_draw(id, wire);
                    let height_t = (display.y / 5.0).clamp(0.0, 1.0);
                    let coin_color = [
                        0.95 * height_t + 0.15,
                        0.75 * (1.0 - height_t) + 0.25,
                        0.2,
                        1.0,
                    ];
                    let mesh_uniforms = self.draw_uniforms_for(
                        view_proj,
                        id,
                        display,
                        pred_override,
                        Vec3::splat(visual_scale),
                        mesh_tilt,
                        coin_color,
                    );
                    self.queue.write_buffer(
                        &self.mesh_uniform_buffers[slot],
                        0,
                        bytemuck::bytes_of(&mesh_uniforms),
                    );
                    pass.set_bind_group(0, &self.mesh_bind_groups[slot], &[]);
                    pass.draw_indexed(0..self.index_count, 0, 0..1);
                }
            }

            // Cuboid boxes (`half_extent > 0`) + remote user_roots: unit cube mesh.
            // Grab highlight uses `SCENE_FLAG_GRABBABLE`; draw is independent of flags (x0027).
            // Per-slot UBOs — rewriting one buffer inside the pass would show only the last cube.
            let local_root = self.shared.local_user_root();
            type PoseList = Vec<(u32, EntityPose)>;
            let box_defs = self.shared.box_defs_snapshot();
            let (boxes, remotes): (PoseList, PoseList) = self
                .shared
                .world
                .lock()
                .ok()
                .map(|w| {
                    let boxes: PoseList = w
                        .entities
                        .iter()
                        .filter(|(id, _)| box_defs.half_extents.contains_key(*id))
                        .map(|(id, p)| (*id, *p))
                        .collect();
                    let remotes: PoseList = w
                        .entities
                        .iter()
                        .filter(|(id, _)| {
                            is_user_root_entity(**id)
                                && (local_root == LOCAL_USER_ROOT_UNSET || **id != local_root)
                        })
                        .map(|(id, p)| (*id, *p))
                        .collect();
                    (boxes, remotes)
                })
                .unwrap_or_default();
            pass.set_vertex_buffer(0, self.box_vertex_buffer.slice(..));
            pass.set_index_buffer(self.box_index_buffer.slice(..), wgpu::IndexFormat::Uint32);
            // Reserve UBO slots for remotes drawn this frame (wire entity count; local excluded).
            let remote_reserve = remotes.len().min(BOX_DRAW_SLOTS);
            let box_slots = BOX_DRAW_SLOTS.saturating_sub(remote_reserve);
            let identity_rot = glam::Quat::IDENTITY;
            for (draw_i, (id, wire)) in boxes.into_iter().enumerate() {
                if draw_i >= box_slots {
                    break;
                }
                let slot = draw_i;
                let he = box_defs
                    .half_extents
                    .get(&id)
                    .copied()
                    .unwrap_or(BOX_HALF_EXTENT);
                let (display, pred_override) = self.shared.display_pose_for_draw(id, wire);
                let mut color = wire.color();
                if highlight_entity == Some(id) {
                    // Brighten so look-aim / Grab target is obvious without a crosshair.
                    color = [
                        (color[0] * 0.35 + 0.65).min(1.0),
                        (color[1] * 0.35 + 0.65).min(1.0),
                        (color[2] * 0.35 + 0.65).min(1.0),
                        1.0,
                    ];
                }
                let box_uniforms = self.draw_uniforms_for(
                    view_proj,
                    id,
                    display,
                    pred_override,
                    Vec3::splat(he * 2.0),
                    identity_rot,
                    color,
                );
                self.queue.write_buffer(
                    &self.box_uniform_buffers[slot],
                    0,
                    bytemuck::bytes_of(&box_uniforms),
                );
                pass.set_bind_group(0, &self.box_bind_groups[slot], &[]);
                pass.draw_indexed(0..self.box_index_count, 0, 0..1);
            }
            // Remote players: reuse box mesh + last UBO slots.
            for (i, (id, pose)) in remotes.iter().enumerate() {
                if i >= remote_reserve || BOX_DRAW_SLOTS <= i {
                    break;
                }
                let slot = BOX_DRAW_SLOTS - 1 - i;
                let box_uniforms = self.draw_uniforms_for(
                    view_proj,
                    *id,
                    *pose,
                    false,
                    Vec3::splat(REMOTE_ROOT_HALF_EXTENT * 2.0),
                    identity_rot,
                    pose.color(),
                );
                self.queue.write_buffer(
                    &self.box_uniform_buffers[slot],
                    0,
                    bytemuck::bytes_of(&box_uniforms),
                );
                pass.set_bind_group(0, &self.box_bind_groups[slot], &[]);
                pass.draw_indexed(0..self.box_index_count, 0, 0..1);
            }
        }
    }
}

fn default_eye_pose() -> EntityPose {
    EntityPose {
        x: 0.0,
        y: DEFAULT_EYE_Y,
        z: 6.0,
        ..Default::default()
    }
}

/// Map local user_root id + wire + optional display freeze → FPS eye TRS (x0092).
///
/// Pure helper so unset / lerp / wire-fallback are unit-tested without constructing
/// full [`SharedState`]. Callers must not treat `wire` alone as view authority when a
/// freeze row exists.
fn resolve_user_root_display_pose(
    local_id: u32,
    wire: EntityPose,
    frame: Option<&crate::epoch_view::DisplayFrame>,
) -> EntityPose {
    if local_id == LOCAL_USER_ROOT_UNSET {
        return wire;
    }
    frame
        .and_then(|f| f.poses.get(&local_id))
        .map(|out| entity_pose_from_gpu_row(out, wire))
        .unwrap_or(wire)
}

/// Hit only when cached anchor id still equals current `local_user_root`.
fn eye_cache_hit(cached: Option<(u32, EntityPose)>, current_id: u32) -> Option<EntityPose> {
    match cached {
        Some((id, pose)) if id == current_id => Some(pose),
        _ => None,
    }
}

/// Display TRS from frozen GPU row + wire color channels (x0091 / x0092 eye).
fn entity_pose_from_gpu_row(out: &crate::epoch_view::GpuEpochRow, wire: EntityPose) -> EntityPose {
    EntityPose {
        x: out.pos[0],
        y: out.pos[1],
        z: out.pos[2],
        qx: out.rot[0],
        qy: out.rot[1],
        qz: out.rot[2],
        qw: out.rot[3],
        sx: out.scale[0],
        sy: out.scale[1],
        sz: out.scale[2],
        r: wire.r,
        g: wire.g,
        b: wire.b,
        a: wire.a,
    }
}

fn make_draw_bind_group(
    device: &wgpu::Device,
    layout: &wgpu::BindGroupLayout,
    uniform: &wgpu::Buffer,
    epoch_out: &wgpu::Buffer,
    label: &str,
) -> wgpu::BindGroup {
    device.create_bind_group(&wgpu::BindGroupDescriptor {
        label: Some(label),
        layout,
        entries: &[
            wgpu::BindGroupEntry {
                binding: 0,
                resource: uniform.as_entire_binding(),
            },
            wgpu::BindGroupEntry {
                binding: 1,
                resource: epoch_out.as_entire_binding(),
            },
        ],
    })
}

/// Axis-aligned unit cube centered at origin (half-extent 0.5 → scale by 2*half).
fn unit_cube_mesh() -> (Vec<MeshVertex>, Vec<u32>) {
    let positions = [
        [-0.5, -0.5, -0.5],
        [0.5, -0.5, -0.5],
        [0.5, 0.5, -0.5],
        [-0.5, 0.5, -0.5],
        [-0.5, -0.5, 0.5],
        [0.5, -0.5, 0.5],
        [0.5, 0.5, 0.5],
        [-0.5, 0.5, 0.5],
    ];
    let indices = [
        0, 1, 2, 0, 2, 3, // -Z
        4, 6, 5, 4, 7, 6, // +Z
        0, 4, 5, 0, 5, 1, // -Y
        3, 2, 6, 3, 6, 7, // +Y
        0, 3, 7, 0, 7, 4, // -X
        1, 5, 6, 1, 6, 2, // +X
    ];
    (
        positions
            .iter()
            .map(|p| MeshVertex { position: *p })
            .collect(),
        indices.to_vec(),
    )
}

pub fn create_depth_view(device: &wgpu::Device, width: u32, height: u32) -> wgpu::TextureView {
    let texture = device.create_texture(&wgpu::TextureDescriptor {
        label: Some("depth"),
        size: wgpu::Extent3d {
            width: width.max(1),
            height: height.max(1),
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: wgpu::TextureFormat::Depth32Float,
        usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
        view_formats: &[],
    });
    texture.create_view(&wgpu::TextureViewDescriptor::default())
}

pub async fn create_wgpu_for_window(
    window: Arc<Window>,
) -> Result<(
    wgpu::Instance,
    wgpu::Surface<'static>,
    wgpu::Adapter,
    wgpu::Device,
    wgpu::Queue,
    wgpu::SurfaceConfiguration,
    wgpu::TextureFormat,
)> {
    let size = window.inner_size();
    let instance = wgpu::Instance::new(&wgpu::InstanceDescriptor {
        backends: wgpu::Backends::VULKAN | wgpu::Backends::DX12 | wgpu::Backends::METAL,
        ..Default::default()
    });
    let surface = instance.create_surface(window.clone())?;
    let adapter = instance
        .request_adapter(&wgpu::RequestAdapterOptions {
            power_preference: wgpu::PowerPreference::HighPerformance,
            compatible_surface: Some(&surface),
            force_fallback_adapter: false,
        })
        .await
        .context("no wgpu adapter")?;
    let (device, queue) = adapter
        .request_device(
            &wgpu::DeviceDescriptor {
                label: Some("alchemy-client"),
                required_features: wgpu::Features::empty(),
                required_limits: wgpu::Limits::default(),
                memory_hints: Default::default(),
            },
            None,
        )
        .await
        .context("request_device")?;

    let caps = surface.get_capabilities(&adapter);
    let format = caps
        .formats
        .iter()
        .copied()
        .find(|f| f.is_srgb())
        .unwrap_or(caps.formats[0]);

    let config = wgpu::SurfaceConfiguration {
        usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
        format,
        width: size.width.max(1),
        height: size.height.max(1),
        present_mode: wgpu::PresentMode::Fifo,
        alpha_mode: caps.alpha_modes[0],
        view_formats: vec![],
        desired_maximum_frame_latency: 2,
    };
    surface.configure(&device, &config);
    Ok((instance, surface, adapter, device, queue, config, format))
}

#[cfg(test)]
mod uniforms_layout_tests {
    use super::Uniforms;

    #[test]
    fn uniforms_match_wgsl_uniform_stride() {
        // view_proj(64)+color(16)+scale_mul+pack(16)+extra_rot(16)+model(64)
        assert_eq!(std::mem::size_of::<Uniforms>(), 176);
        assert_eq!(std::mem::align_of::<Uniforms>(), 4);
    }
}

#[cfg(test)]
mod eye_display_tests {
    use std::sync::Arc;

    use alchemy_shared_sim::bridge::EpochFaces;
    use alchemy_shared_sim::{EpochSheet, WorldTransformf64};

    use super::{
        default_eye_pose, eye_cache_hit, resolve_user_root_display_pose, EntityPose,
        LOCAL_USER_ROOT_UNSET,
    };
    use crate::epoch_mirror::{published_instant, EPOCH_FRAME_PERIOD};
    use crate::epoch_view::build_display_frame;

    fn mid_alpha_frame(entity_id: u32) -> Arc<crate::epoch_view::DisplayFrame> {
        let mut prev = EpochSheet::with_capacity(1);
        let h = prev.alloc_row();
        prev.owner_scope[h.row_id as usize] = entity_id;
        prev.set_transform(h.row_id, WorldTransformf64::identity_at([0.0, 1.6, 0.0]))
            .unwrap();
        let mut curr = prev.clone();
        curr.set_transform(h.row_id, WorldTransformf64::identity_at([10.0, 1.6, 0.0]))
            .unwrap();

        let published_at_ns = 10_000_000u64;
        let faces = Arc::new(EpochFaces {
            prev: Arc::new(prev),
            curr: Arc::new(curr),
            published_at_ns,
        });
        let now = published_instant(published_at_ns).unwrap() + EPOCH_FRAME_PERIOD / 2;
        let frame = build_display_frame(faces, now);
        assert!((frame.alpha - 0.5).abs() < 0.05, "alpha={}", frame.alpha);
        frame
    }

    #[test]
    fn eye_trs_follows_display_lerp_not_wire_curr() {
        let frame = mid_alpha_frame(7);
        let wire = EntityPose {
            x: 10.0,
            y: 1.6,
            z: 0.0,
            r: 0.25,
            ..Default::default()
        };
        let eye = resolve_user_root_display_pose(7, wire, Some(&frame));
        assert!(
            (eye.x - 5.0).abs() < 0.15,
            "eye should be mid Prev/Curr, got {}",
            eye.x
        );
        assert!(
            (eye.x - wire.x).abs() > 1.0,
            "eye must not equal wire Curr alone"
        );
        assert_eq!(eye.r, wire.r, "color stays on wire");
    }

    #[test]
    fn unset_local_user_root_keeps_default_eye_wire() {
        let frame = mid_alpha_frame(7);
        let wire = default_eye_pose();
        let eye = resolve_user_root_display_pose(LOCAL_USER_ROOT_UNSET, wire, Some(&frame));
        assert_eq!(eye, wire);
        assert!((eye.y - 1.6).abs() < 1e-5);
        assert!((eye.z - 6.0).abs() < 1e-5);
    }

    #[test]
    fn missing_display_row_falls_back_to_wire() {
        let frame = mid_alpha_frame(7);
        let wire = EntityPose {
            x: 3.0,
            y: 1.6,
            z: 9.0,
            ..Default::default()
        };
        let eye = resolve_user_root_display_pose(99, wire, Some(&frame));
        assert_eq!(eye, wire);
    }

    #[test]
    fn no_frame_falls_back_to_wire() {
        let wire = EntityPose {
            x: 1.0,
            y: 2.0,
            z: 3.0,
            ..Default::default()
        };
        let eye = resolve_user_root_display_pose(7, wire, None);
        assert_eq!(eye, wire);
    }

    #[test]
    fn eye_cache_hit_requires_matching_anchor_id() {
        let pose = EntityPose {
            x: 4.0,
            y: 1.6,
            z: 2.0,
            ..Default::default()
        };
        assert_eq!(eye_cache_hit(Some((7, pose)), 7), Some(pose));
        assert_eq!(eye_cache_hit(Some((7, pose)), 8), None);
        assert_eq!(eye_cache_hit(None, 7), None);
    }

    /// XR / Desktop contract: freeze before reading eye (regression lock on source order).
    #[test]
    fn xr_session_freezes_before_user_root_pose() {
        let src = include_str!("surface/xr/wgpu_session.rs");
        let freeze = src
            .find("shared.begin_display_frame();")
            .expect("begin_display_frame call");
        let eye = src
            .find("shared.user_root_pose();")
            .expect("user_root_pose call");
        assert!(
            freeze < eye,
            "begin_display_frame must precede user_root_pose (x0092 floor/Grab same snap)"
        );
    }
}

#[cfg(test)]
mod refresh_aead_tests {
    use super::{
        clear_pending_unless_newer, commit_pending_into_live, refresh_aead_action,
        take_pending_if_grace_expired, PendingRoomAead, RefreshAeadAction,
    };
    use std::sync::Arc;
    use std::time::{Duration, Instant};

    #[test]
    fn epoch_zero_to_one_prepares_pending() {
        assert_eq!(
            refresh_aead_action(0, 1, false),
            RefreshAeadAction::PreparePending
        );
    }

    #[test]
    fn epoch_gap_greater_than_one_is_key_stale() {
        assert_eq!(
            refresh_aead_action(0, 2, false),
            RefreshAeadAction::KeyStale
        );
        assert_eq!(
            refresh_aead_action(3, 5, false),
            RefreshAeadAction::KeyStale
        );
    }

    #[test]
    fn epoch_advance_while_hard_stale_stays_key_stale() {
        assert_eq!(refresh_aead_action(0, 1, true), RefreshAeadAction::KeyStale);
    }

    #[test]
    fn same_epoch_commits_when_not_stale() {
        assert_eq!(refresh_aead_action(3, 3, false), RefreshAeadAction::Commit);
    }

    #[test]
    fn same_epoch_does_not_clear_stale() {
        assert_eq!(
            refresh_aead_action(3, 3, true),
            RefreshAeadAction::IgnoreWhileStale
        );
    }

    #[test]
    fn backward_epoch_is_ignored() {
        assert_eq!(
            refresh_aead_action(5, 4, false),
            RefreshAeadAction::IgnoreBackward
        );
    }

    #[test]
    fn commit_pending_requires_bus_and_live_key() {
        let key = Arc::new([9u8; alchemy_protocol::wire_aead::ROOM_KEY_LEN]);
        let mut pending = Some(PendingRoomAead {
            key: Arc::clone(&key),
            epoch: 4,
            deadline: Instant::now() + Duration::from_secs(60),
        });
        let mut live = None;
        assert!(
            commit_pending_into_live(&mut live, &mut pending, 4, true).is_none(),
            "Refuse write-back after Leave cleared live"
        );
        assert!(pending.is_some(), "pending must survive failed commit");

        live = Some(Arc::new([1u8; alchemy_protocol::wire_aead::ROOM_KEY_LEN]));
        assert!(
            commit_pending_into_live(&mut live, &mut pending, 4, false).is_none(),
            "Refuse when room bus is down"
        );
        assert!(pending.is_some());
    }

    #[test]
    fn commit_pending_clears_only_after_live_update() {
        let new_key = Arc::new([9u8; alchemy_protocol::wire_aead::ROOM_KEY_LEN]);
        let mut live = Some(Arc::new([1u8; alchemy_protocol::wire_aead::ROOM_KEY_LEN]));
        let mut pending = Some(PendingRoomAead {
            key: Arc::clone(&new_key),
            epoch: 4,
            deadline: Instant::now() + Duration::from_secs(60),
        });
        let (installed, epoch) =
            commit_pending_into_live(&mut live, &mut pending, 4, true).expect("commit");
        assert_eq!(epoch, 4);
        assert_eq!(installed.as_ref(), new_key.as_ref());
        assert_eq!(live.as_ref().map(|k| k.as_ref()), Some(new_key.as_ref()));
        assert!(pending.is_none());
    }

    #[test]
    fn commit_pending_rejects_epoch_mismatch_without_clearing() {
        let mut live = Some(Arc::new([1u8; alchemy_protocol::wire_aead::ROOM_KEY_LEN]));
        let mut pending = Some(PendingRoomAead {
            key: Arc::new([9u8; alchemy_protocol::wire_aead::ROOM_KEY_LEN]),
            epoch: 4,
            deadline: Instant::now() + Duration::from_secs(60),
        });
        assert!(commit_pending_into_live(&mut live, &mut pending, 3, true).is_none());
        assert!(pending.is_some());
    }

    #[test]
    fn same_epoch_commit_keeps_newer_pending() {
        let mut pending = Some(PendingRoomAead {
            key: Arc::new([9u8; alchemy_protocol::wire_aead::ROOM_KEY_LEN]),
            epoch: 5,
            deadline: Instant::now() + Duration::from_secs(60),
        });
        clear_pending_unless_newer(&mut pending, 4);
        assert_eq!(
            pending.as_ref().map(|p| p.epoch),
            Some(5),
            "same-epoch Commit must not wipe pending ahead of live (x00d5 §8)"
        );

        clear_pending_unless_newer(&mut pending, 5);
        assert!(
            pending.is_none(),
            "equal epoch may clear (stash already applied)"
        );

        pending = Some(PendingRoomAead {
            key: Arc::new([8u8; alchemy_protocol::wire_aead::ROOM_KEY_LEN]),
            epoch: 3,
            deadline: Instant::now() + Duration::from_secs(60),
        });
        clear_pending_unless_newer(&mut pending, 4);
        assert!(
            pending.is_none(),
            "stale pending behind commit epoch is cleared"
        );
    }

    #[test]
    fn grace_expired_takes_pending_once() {
        let mut pending = Some(PendingRoomAead {
            key: Arc::new([9u8; alchemy_protocol::wire_aead::ROOM_KEY_LEN]),
            epoch: 7,
            deadline: Instant::now() - Duration::from_secs(1),
        });
        assert_eq!(
            take_pending_if_grace_expired(&mut pending, Instant::now()),
            Some(7)
        );
        assert!(pending.is_none());
        assert_eq!(
            take_pending_if_grace_expired(&mut pending, Instant::now()),
            None,
            "second poll must not KeyStale after commit already cleared pending"
        );
    }

    #[test]
    fn grace_not_expired_keeps_pending() {
        let mut pending = Some(PendingRoomAead {
            key: Arc::new([9u8; alchemy_protocol::wire_aead::ROOM_KEY_LEN]),
            epoch: 7,
            deadline: Instant::now() + Duration::from_secs(60),
        });
        assert_eq!(
            take_pending_if_grace_expired(&mut pending, Instant::now()),
            None
        );
        assert!(pending.is_some());
    }
}
