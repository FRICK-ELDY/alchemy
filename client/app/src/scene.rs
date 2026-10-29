//! Shared scene GPU (Coin + floor + grabbable boxes + remote user_roots).
//!
//! MoveIntent carries Grab hold fields for x0011 (host kinematic follow).
//! `CameraLook` is Desktop-local (x0012); not published on Zenoh.
//!
//! Guarantees: draw with caller-supplied view/proj; grabbable boxes from Manifest (x0027);
//! remote roots 3..6 (x0015); Frame TRS on draw (x0029);
//! [`SharedState::asset_status`] sticky Home overlay for auth asset failure (x0048).
//! Non-guarantees: XR swapchain ownership, system_ui layout, avatar GLB;
//! live mesh swap after boot.
//! Grab hits use OBB in `world_hit` (x002b; draw TRS aligned).
//! Leave / liveliness token ownership lives here; put/undeclare is `main::leave_room`.

use std::collections::{HashMap, HashSet};
use std::sync::atomic::{AtomicU32, Ordering};
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
/// Default half-extent when Manifest omits per-entity size.
pub use alchemy_shared_sim::DEFAULT_BOX_HALF_EXTENT as BOX_HALF_EXTENT;
/// Placeholder size for remote player (meters half-extent).
pub const REMOTE_ROOT_HALF_EXTENT: f32 = 0.25;

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

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
pub struct Uniforms {
    pub mvp: [[f32; 4]; 4],
    pub color: [f32; 4],
}

/// Pose table + tick gate under one lock (x0023: snapshot/delta race-free).
#[derive(Default)]
pub struct WorldView {
    pub last_frame_tick: Option<u64>,
    pub entities: std::collections::HashMap<u32, EntityPose>,
}

pub struct SharedState {
    /// Local camera / aim follows this host entity (`LOCAL_USER_ROOT_UNSET` until Presence).
    pub local_user_root: AtomicU32,
    /// `ROOM_ID` used for Zenoh keys (x0017); shared with host_control shutdown publish.
    pub room_id: Arc<str>,
    pub auth_url: Arc<str>,
    pub world_url: Arc<str>,
    pub assets_url: Arc<str>,
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
    /// Last accepted Presence watermark (x003a); generation must match Manifest.
    pub presence_gen_rev: Mutex<Option<alchemy_protocol::generation_gate::GenRev>>,
    /// Zenoh liveliness token while joined (x0022 `sub`+`jti`). Drop/undeclare on leave.
    pub liveliness: Mutex<Option<zenoh::liveliness::LivelinessToken>>,
    /// Previous liveliness key retained one Intent put after refresh rebind (x0022).
    pub liveliness_prev: Mutex<Option<zenoh::liveliness::LivelinessToken>>,
    /// Entity poses + last applied frame tick (single mutex).
    pub world: Mutex<WorldView>,
    /// Optional localhost Prometheus metrics (x0049); always present, server off unless env.
    pub metrics: Arc<crate::metrics::ClientMetrics>,
    /// Manifest cuboid defs (Grab flags + half_extents); Arc-swapped on apply (x0027).
    pub box_defs: Mutex<Arc<BoxDefs>>,
    /// Primary mesh entity id from Manifest (`blake3` entry); 0 until set.
    pub mesh_entity_id: AtomicU32,
    /// Authenticated asset-load failure message for dash Home status (x0048).
    /// `None` when mesh came from assets GET or unauthenticated Coin Home fixture.
    pub asset_status: Mutex<Option<String>>,
    pub look: Mutex<CameraLook>,
    pub intent: Mutex<MoveIntent>,
}

impl SharedState {
    /// Sticky Home status: if an asset failure is recorded, overwrite `home_status` and
    /// return `true` (caller should keep Home tab open). Skips while status looks busy
    /// (`…` / `...`) so "Signing in…" is not clobbered mid-flight.
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
            return true;
        }
        *home_status = msg.clone();
        true
    }

    pub fn store_asset_status(&self, status: String) {
        let mut guard = self
            .asset_status
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        *guard = Some(status);
    }

    pub fn local_user_root(&self) -> u32 {
        self.local_user_root.load(Ordering::Relaxed)
    }

    pub fn set_local_user_root(&self, entity_id: u32) {
        self.local_user_root.store(entity_id, Ordering::Relaxed)
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

    pub fn leave_room_local(&self) {
        self.set_local_user_root(LOCAL_USER_ROOT_UNSET);
        if let Ok(mut intent) = self.intent.lock() {
            *intent = MoveIntent {
                suppress: intent.suppress,
                ..Default::default()
            };
        }
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
        if let Ok(mut slot) = self.presence_gen_rev.lock() {
            *slot = None;
        }
        if let Ok(mut world) = self.world.lock() {
            *world = WorldView::default();
        }
        if let Ok(mut slot) = self.box_defs.lock() {
            *slot = Arc::new(BoxDefs::default());
        }
        self.mesh_entity_id.store(0, Ordering::Relaxed);
        if let Ok(mut token) = self.liveliness.lock() {
            *token = None;
        }
        if let Ok(mut prev) = self.liveliness_prev.lock() {
            *prev = None;
        }
    }

    /// Host generation change: clear world + Presence watermark (x0023 / x003a).
    /// x003e: mint new Intent key into `pending_intent_session` only; live publisher keeps
    /// old key + old Token until Room Token for the new pub arrives, then **atomic** swap.
    pub fn reset_for_host_generation(&self) {
        if let Ok(mut world) = self.world.lock() {
            *world = WorldView::default();
        }
        if let Ok(mut slot) = self.box_defs.lock() {
            *slot = Arc::new(BoxDefs::default());
        }
        self.mesh_entity_id.store(0, Ordering::Relaxed);
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

        match crate::world_token::fetch_guest_token(
            &self.world_url,
            &self.room_id,
            &access,
            &session_pub_b64,
        ) {
            Ok(guest) => {
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
        auth.apply_room_refresh(guest.token, local_sub, guest.expires_in);
        *sess = Some(new_sess);
        drop(pending);
        drop(sess);
        drop(auth);
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
        let Ok(_pending) = self.pending_intent_session.lock() else {
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
        auth.apply_room_refresh(guest.token, local_sub, guest.expires_in);
        s.mark_bind_pending();
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
    pub fn request_intent_rebind(&self) {
        if let Ok(mut sess) = self.intent_session.lock() {
            if let Some(s) = sess.as_mut() {
                if !s.bind_pending() {
                    s.mark_bind_pending();
                    log::info!("intent rebind pending (Presence missing while steady)");
                }
            }
        }
    }
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
    coin_uniform_buffer: wgpu::Buffer,
    coin_bind_group: wgpu::BindGroup,
    box_vertex_buffer: wgpu::Buffer,
    box_index_buffer: wgpu::Buffer,
    box_index_count: u32,
    /// One uniform+bind group per box slot — wgpu cannot safely rewrite one UBO mid-pass.
    box_uniform_buffers: Vec<wgpu::Buffer>,
    box_bind_groups: Vec<wgpu::BindGroup>,
    mesh_radius: f32,
    shared: Arc<SharedState>,
    last_pose_log: Instant,
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
            label: Some("uniform-bgl"),
            entries: &[wgpu::BindGroupLayoutEntry {
                binding: 0,
                visibility: wgpu::ShaderStages::VERTEX | wgpu::ShaderStages::FRAGMENT,
                ty: wgpu::BindingType::Buffer {
                    ty: wgpu::BufferBindingType::Uniform,
                    has_dynamic_offset: false,
                    min_binding_size: None,
                },
                count: None,
            }],
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
        let coin_uniform_buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("coin-uniforms"),
            size: std::mem::size_of::<Uniforms>() as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });

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
            let bg = device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some(&format!("box-bg-{i}")),
                layout: &bind_group_layout,
                entries: &[wgpu::BindGroupEntry {
                    binding: 0,
                    resource: buf.as_entire_binding(),
                }],
            });
            box_uniform_buffers.push(buf);
            box_bind_groups.push(bg);
        }

        let floor_bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("floor-bg"),
            layout: &bind_group_layout,
            entries: &[wgpu::BindGroupEntry {
                binding: 0,
                resource: floor_uniform_buffer.as_entire_binding(),
            }],
        });
        let coin_bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("coin-bg"),
            layout: &bind_group_layout,
            entries: &[wgpu::BindGroupEntry {
                binding: 0,
                resource: coin_uniform_buffer.as_entire_binding(),
            }],
        });

        Ok(Self {
            device,
            queue,
            pipeline,
            surface_format,
            vertex_buffer,
            index_buffer,
            index_count: mesh.indices.len() as u32,
            floor_vertex_buffer,
            floor_index_buffer,
            floor_index_count: floor_indices.len() as u32,
            floor_uniform_buffer,
            floor_bind_group,
            coin_uniform_buffer,
            coin_bind_group,
            box_vertex_buffer,
            box_index_buffer,
            box_index_count: box_indices.len() as u32,
            box_uniform_buffers,
            box_bind_groups,
            mesh_radius: mesh.radius,
            shared,
            last_pose_log: Instant::now(),
        })
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

    pub fn user_root_pose(&self) -> EntityPose {
        let id = self.shared.local_user_root();
        if id == LOCAL_USER_ROOT_UNSET {
            return EntityPose {
                x: 0.0,
                y: DEFAULT_EYE_Y,
                z: 6.0,
                ..Default::default()
            };
        }
        self.shared
            .world
            .lock()
            .ok()
            .and_then(|w| w.entities.get(&id).copied())
            .unwrap_or(EntityPose {
                x: 0.0,
                y: DEFAULT_EYE_Y,
                z: 6.0,
                ..Default::default()
            })
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
        let pose = self.coin_pose();
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

            let floor_uniforms = Uniforms {
                mvp: (proj * view).to_cols_array_2d(),
                color: [0.22, 0.24, 0.28, 1.0],
            };
            self.queue.write_buffer(
                &self.floor_uniform_buffer,
                0,
                bytemuck::bytes_of(&floor_uniforms),
            );
            pass.set_bind_group(0, &self.floor_bind_group, &[]);
            pass.set_vertex_buffer(0, self.floor_vertex_buffer.slice(..));
            pass.set_index_buffer(self.floor_index_buffer.slice(..), wgpu::IndexFormat::Uint32);
            pass.draw_indexed(0..self.floor_index_count, 0, 0..1);

            let visual_scale = (alchemy_shared_sim::PHYS_BALL_RADIUS * 0.95
                / self.mesh_radius.max(1e-3))
            .clamp(0.02, 4.0);
            // Mesh asset tilt (0.25 rad X) stays client-local; Frame TRS is sim authority (x0029).
            let model = Mat4::from_scale_rotation_translation(
                pose.scale() * visual_scale,
                pose.rotation() * glam::Quat::from_rotation_x(0.25),
                pose.position(),
            );
            let height_t = (pose.y / 5.0).clamp(0.0, 1.0);
            let coin_color = [
                0.95 * height_t + 0.15,
                0.75 * (1.0 - height_t) + 0.25,
                0.2,
                1.0,
            ];
            let coin_uniforms = Uniforms {
                mvp: (proj * view * model).to_cols_array_2d(),
                color: coin_color,
            };
            self.queue.write_buffer(
                &self.coin_uniform_buffer,
                0,
                bytemuck::bytes_of(&coin_uniforms),
            );
            pass.set_bind_group(0, &self.coin_bind_group, &[]);
            pass.set_vertex_buffer(0, self.vertex_buffer.slice(..));
            pass.set_index_buffer(self.index_buffer.slice(..), wgpu::IndexFormat::Uint32);
            pass.draw_indexed(0..self.index_count, 0, 0..1);

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
            // Reserve UBO slots for remote roots (protocol band ceiling; local excluded).
            let remote_reserve = alchemy_protocol::USER_ROOT_SLOTS_MAX.min(BOX_DRAW_SLOTS);
            let box_slots = BOX_DRAW_SLOTS.saturating_sub(remote_reserve);
            for (draw_i, (id, pose)) in boxes.into_iter().enumerate() {
                if draw_i >= box_slots {
                    break;
                }
                let slot = draw_i;
                let he = box_defs
                    .half_extents
                    .get(&id)
                    .copied()
                    .unwrap_or(BOX_HALF_EXTENT);
                let model = Mat4::from_scale_rotation_translation(
                    pose.scale() * (he * 2.0),
                    pose.rotation(),
                    pose.position(),
                );
                let mut color = pose.color();
                if highlight_entity == Some(id) {
                    // Brighten so look-aim / Grab target is obvious without a crosshair.
                    color = [
                        (color[0] * 0.35 + 0.65).min(1.0),
                        (color[1] * 0.35 + 0.65).min(1.0),
                        (color[2] * 0.35 + 0.65).min(1.0),
                        1.0,
                    ];
                }
                let box_uniforms = Uniforms {
                    mvp: (proj * view * model).to_cols_array_2d(),
                    color,
                };
                self.queue.write_buffer(
                    &self.box_uniform_buffers[slot],
                    0,
                    bytemuck::bytes_of(&box_uniforms),
                );
                pass.set_bind_group(0, &self.box_bind_groups[slot], &[]);
                pass.draw_indexed(0..self.box_index_count, 0, 0..1);
            }
            // Remote players: reuse box mesh + last UBO slots.
            for (i, (_id, pose)) in remotes.iter().enumerate() {
                if i >= remote_reserve || BOX_DRAW_SLOTS <= i {
                    break;
                }
                let slot = BOX_DRAW_SLOTS - 1 - i;
                let model = Mat4::from_scale_rotation_translation(
                    pose.scale() * (REMOTE_ROOT_HALF_EXTENT * 2.0),
                    pose.rotation(),
                    pose.position(),
                );
                let box_uniforms = Uniforms {
                    mvp: (proj * view * model).to_cols_array_2d(),
                    color: pose.color(),
                };
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
