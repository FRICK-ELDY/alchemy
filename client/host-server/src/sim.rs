//! Epoch/XPBD world driven by SceneManifest entries (x0093; was Rapier through x008f).
//!
//! Guarantees: ground plane (y=0) + mesh ball + boxes + user_roots from Manifest;
//! Manifest initial **rotation** / **scale** seed Frame Pose; per-slot Grab; no Steal;
//! hold aim / offset capped by [`alchemy_shared_sim::MAX_GRAB_HIT_DISTANCE`] (x0041);
//! Grab on/off syncs [`alchemy_shared_sim::SlotGraph`] held parent (x0057);
//! `release_slot` resets translation+rotation to Manifest spawn;
//! Presence capacity = contiguous Manifest user_root entries (x004d / x009b);
//! wire TRS authority is Epoch Curr after [`Self::step`] (x0093);
//! **x00ad**: [`SimBackendKind`] switch; LBVH body-body contacts **on** (`bounds: Some`);
//! `mvcc_xpbd` is a **temporary bridge** (page steps XPBD; wire still samples Epoch Curr).
//! Classification via `alchemy-shared-sim` [`EntityKind`] (x0030).
//! Non-guarantees: Grab aim **rotation** follow (translation only); throw impulse;
//! Rapier bit-exact parity; capsule Walk extrude; host dirty 30Hz 本番切替 / PROTOCOL_PIN bump;
//! (shared-sim `delta_codec` is x00ae; `…/sim_state` export/import is x00b0);
//! non-uniform collider scale; Frame parent fields / pass-place Intent wiring;
//! exact OBB box-box (sphere approx via LBVH); production default `mvcc_xpbd`;
//! `mvcc_xpbd` cell_origin always 0（近傍デモ向け；遠方は f32 量子化）;
//! sheet↔page 毎 tick 往復の zero-alloc（後続 dirty 前に計測）;
//! Epoch 二重管理の解消（dirty 本番切替で畳む）; `drot` 復元（予約 0）.

use std::collections::{HashMap, HashSet};

#[cfg(test)]
use alchemy_protocol::Message;
use alchemy_protocol::{
    entity_holder_from_slot, entity_holder_slot, scene_entry_grabbable, scene_entry_quat_xyzw,
    scene_entry_scale, user_root_slots_from_entries, LocomotionMode, SceneEntry, SimEntity,
    SimState, DEFAULT_MOVE_SPEED, ENTITY_HOLDER_NONE, PROTOCOL_PIN,
};
#[cfg(test)]
use alchemy_shared_sim::mvcc::KinematicPoseF32;
use alchemy_shared_sim::mvcc::{HotPhysicsPage, PageXpbdAdapter, ENTITIES_PER_PAGE, NO_HOLDER};
use alchemy_shared_sim::spatial::SceneBounds;
use alchemy_shared_sim::xpbd::{self, XpbdWorld};
use alchemy_shared_sim::{
    classify_scene_entry, scene_entry_color_rgba, ColliderShape, EntityKind, EpochSheet,
    EpochSnapshotPair, FlatTopology, NodeHandle, ReadinessMask, SlotGraph, WorldTransformf64,
    PHYS_BALL_RADIUS,
};

pub use crate::sim_backend::SimBackendKind;
pub use alchemy_protocol::{USER_ROOT_ENTITY_FIRST, USER_ROOT_SLOTS_MAX};

const USER_ROOT_SENSOR_RADIUS: f32 = 0.3;
const DEMO_GRAVITY_Y: f64 = -2.2;
const DEFAULT_DT: f64 = 1.0 / 60.0;
const GROUND_Y: f64 = 0.0;

/// Page-path XPBD state for [`SimBackendKind::MvccXpbd`] (x00ad).
struct HostMvccState {
    adapter: PageXpbdAdapter,
    page: HotPhysicsPage,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LocoMode {
    Noclip,
    Fly,
    Walk,
}

impl LocoMode {
    pub fn from_proto(mode: i32) -> Self {
        match LocomotionMode::try_from(mode).unwrap_or(LocomotionMode::Unspecified) {
            LocomotionMode::Fly => Self::Fly,
            LocomotionMode::Walk => Self::Walk,
            LocomotionMode::Noclip | LocomotionMode::Unspecified => Self::Noclip,
        }
    }
}

/// Cuboids from Manifest (`half_extent > 0`). Grab is per-entry via `grabbable`.
pub struct CuboidBoxes {
    pub entity_ids: Vec<u32>,
    pub grabbable: Vec<bool>,
    pub half_extents: Vec<f32>,
}

impl CuboidBoxes {
    fn empty() -> Self {
        Self {
            entity_ids: Vec::new(),
            grabbable: Vec::new(),
            half_extents: Vec::new(),
        }
    }

    pub fn len(&self) -> usize {
        self.entity_ids.len()
    }
}

#[derive(Clone, Copy, Debug)]
struct ActiveGrab {
    index: usize,
    offset: [f32; 3],
}

/// Authoritative host sim: EpochSheet + XPBD (x0093 / x00ad backend switch).
pub struct PhysicsWorld {
    world: XpbdWorld,
    topo: FlatTopology,
    snapshots: EpochSnapshotPair,
    entity_rows: HashMap<u32, NodeHandle>,
    meshes: Vec<u32>,
    user_root_rows: [Option<NodeHandle>; USER_ROOT_SLOTS_MAX],
    user_root_spawns: [(f32, f32, f32); USER_ROOT_SLOTS_MAX],
    user_root_quats: [[f32; 4]; USER_ROOT_SLOTS_MAX],
    slot_count: usize,
    scales: HashMap<u32, [f32; 3]>,
    pub boxes: CuboidBoxes,
    colors: HashMap<u32, (f32, f32, f32, f32)>,
    loco_mode: LocoMode,
    move_speed: f32,
    grabs: [Option<ActiveGrab>; USER_ROOT_SLOTS_MAX],
    /// Last aim used while holding (world space); drives grab_targets each tick.
    grab_aims: [[f32; 3]; USER_ROOT_SLOTS_MAX],
    slots: SlotGraph,
    /// Curr mutated outside `step_dt` (Intent move / release / spawn…) — force snapshot publish.
    publish_dirty: bool,
    /// Reused by [`Self::resolve_ground_plane`] (avoid per-tick HashSet/Vec alloc).
    ground_scratch: HashSet<u32>,
    /// Rows reset by [`Self::sanitize_non_finite_dynamics`] this tick (force delta = 0).
    sanitize_scratch: HashSet<u32>,
    active_row_scratch: Vec<u32>,
    backend: SimBackendKind,
    scene_bounds: SceneBounds,
    mvcc: Option<HostMvccState>,
    pub barrier_ticks: u64,
    pub publishes: u64,
}

fn entry_quat_xyzw(entry: &SceneEntry) -> [f32; 4] {
    let [qx, qy, qz, qw] = scene_entry_quat_xyzw(entry);
    [qx as f32, qy as f32, qz as f32, qw as f32]
}

fn entry_scale_f32(entry: &SceneEntry) -> [f32; 3] {
    let [sx, sy, sz] = scene_entry_scale(entry);
    [sx as f32, sy as f32, sz as f32]
}

fn xf_at(pos: [f64; 3], rot: [f32; 4], scale: [f32; 3], inv_mass: f64) -> WorldTransformf64 {
    WorldTransformf64 {
        position: pos,
        inv_mass,
        rotation: rot,
        scale,
        frame_delta_xyz: [0.0, 0.0, 0.0],
        flags: 0,
        _pad0: 0,
        _pad1: [0; 4],
    }
}

fn support_half_y(shape: &ColliderShape) -> f64 {
    match shape.shape_type {
        ColliderShape::SPHERE => shape.extents[0] as f64,
        ColliderShape::CAPSULE => (shape.extents[0] + shape.extents[1]) as f64,
        ColliderShape::BOX => shape.extents[1] as f64,
        _ => shape.extents[0] as f64,
    }
}

impl PhysicsWorld {
    /// Build physics from world-authored Manifest entries (x0027 / x004d / x0093).
    /// Uses [`SimBackendKind::EpochXpbd`] (tests / callers that do not select a backend).
    #[cfg_attr(not(test), allow(dead_code))]
    pub fn from_entries(entries: &[SceneEntry], loco_mode: LocoMode, move_speed: f32) -> Self {
        Self::from_entries_with_backend(entries, loco_mode, move_speed, SimBackendKind::EpochXpbd)
            .unwrap_or_else(|e| panic!("{e}"))
    }

    /// Like [`Self::from_entries`], with explicit [`SimBackendKind`] (x00ad).
    ///
    /// Failures (capacity / invalid Manifest roots) are `Err` — same class as bad
    /// `ALCHEMY_SIM_BACKEND` (startup failure, not silent fallback).
    pub fn from_entries_with_backend(
        entries: &[SceneEntry],
        loco_mode: LocoMode,
        move_speed: f32,
        backend: SimBackendKind,
    ) -> Result<Self, String> {
        let slots = user_root_slots_from_entries(entries).map_err(|e| e.to_string())?;

        let mut colors = HashMap::new();
        let mut scales: HashMap<u32, [f32; 3]> = HashMap::new();
        let mut boxes = CuboidBoxes::empty();
        let mut meshes: Vec<u32> = Vec::new();
        let mut user_root_spawns = [(0.0_f32, 1.6_f32, 6.0_f32); USER_ROOT_SLOTS_MAX];
        let mut user_root_quats = [[0.0_f32, 0.0, 0.0, 1.0]; USER_ROOT_SLOTS_MAX];
        let mut roots_seen = [false; USER_ROOT_SLOTS_MAX];

        let mut world = XpbdWorld::new(entries.len() + 8);
        world.gravity = [0.0, DEMO_GRAVITY_Y, 0.0];
        world.substeps = 4;
        let mut entity_rows = HashMap::new();

        for entry in entries {
            colors.insert(entry.entity_id, scene_entry_color_rgba(entry));
            scales.insert(entry.entity_id, entry_scale_f32(entry));
            let rot = entry_quat_xyzw(entry);
            let scale = entry_scale_f32(entry);
            let pos = [entry.x, entry.y, entry.z];

            match classify_scene_entry(entry) {
                EntityKind::UserRoot => {
                    let slot = (entry.entity_id - USER_ROOT_ENTITY_FIRST) as usize;
                    if slot < slots {
                        user_root_spawns[slot] = (entry.x as f32, entry.y as f32, entry.z as f32);
                        user_root_quats[slot] = rot;
                        roots_seen[slot] = true;
                    }
                }
                EntityKind::Mesh => {
                    if entry.half_extent > 0.0 {
                        log::warn!(
                            "scene entry {} has blake3 + half_extent; mesh wins (box ignored)",
                            entry.entity_id
                        );
                    }
                    if meshes.contains(&entry.entity_id) {
                        log::warn!(
                            "duplicate mesh entity_id {}; skipping collider",
                            entry.entity_id
                        );
                        continue;
                    }
                    let handle = alloc_dynamic_sphere(
                        &mut world,
                        pos,
                        rot,
                        scale,
                        entry.entity_id,
                        PHYS_BALL_RADIUS,
                        true,
                    );
                    entity_rows.insert(entry.entity_id, handle);
                    meshes.push(entry.entity_id);
                }
                EntityKind::Box => {
                    let he = entry.half_extent;
                    let can_grab = scene_entry_grabbable(entry.flags);
                    let handle = world.curr.alloc_row();
                    let inv_mass = if can_grab { 1.0 } else { 0.0 };
                    let xf = xf_at(pos, rot, scale, inv_mass);
                    let _ = world.curr.set_transform(handle.row_id, xf);
                    world.curr.colliders[handle.row_id as usize] = ColliderShape {
                        shape_type: ColliderShape::BOX,
                        extents: [he, he, he],
                    };
                    let mut mask = ReadinessMask(
                        ReadinessMask::ALLOCATED
                            | ReadinessMask::TRANSFORM
                            | ReadinessMask::COLLIDER,
                    );
                    if can_grab {
                        mask = mask.with_bits(ReadinessMask::DYNAMIC);
                    }
                    world.curr.readiness[handle.row_id as usize] = mask;
                    world.curr.owner_scope[handle.row_id as usize] = entry.entity_id;
                    entity_rows.insert(entry.entity_id, handle);
                    boxes.entity_ids.push(entry.entity_id);
                    boxes.grabbable.push(can_grab);
                    boxes.half_extents.push(he);
                }
                EntityKind::Ignored => {}
            }
        }

        let mut user_root_rows = [None; USER_ROOT_SLOTS_MAX];
        for slot in 0..slots {
            let (ux, uy, uz) = user_root_spawns[slot];
            let rot = user_root_quats[slot];
            let id = USER_ROOT_ENTITY_FIRST + slot as u32;
            let handle = world.curr.alloc_row();
            let scale = scales.get(&id).copied().unwrap_or([1.0, 1.0, 1.0]);
            let xf = xf_at([ux as f64, uy as f64, uz as f64], rot, scale, 0.0);
            let _ = world.curr.set_transform(handle.row_id, xf);
            world.curr.colliders[handle.row_id as usize] = ColliderShape {
                shape_type: ColliderShape::SPHERE,
                extents: [USER_ROOT_SENSOR_RADIUS, 0.0, 0.0],
            };
            world.curr.readiness[handle.row_id as usize] = ReadinessMask(
                ReadinessMask::ALLOCATED | ReadinessMask::TRANSFORM | ReadinessMask::COLLIDER,
            );
            world.curr.owner_scope[handle.row_id as usize] = id;
            entity_rows.insert(id, handle);
            user_root_rows[slot] = Some(handle);
            scales.entry(id).or_insert([1.0, 1.0, 1.0]);
            if !roots_seen[slot] {
                colors.insert(id, user_root_color(slot));
            }
        }

        xpbd::sync_prev_capacity(&mut world);
        world.prev = world.curr.clone();
        // Initial horizontal kick via frame_delta (survives commit; x0093 Verlet handoff).
        for &id in &meshes {
            if let Some(h) = entity_rows.get(&id) {
                let i = h.row_id as usize;
                world.curr.world_transforms[i].frame_delta_xyz =
                    [0.1 * DEFAULT_DT as f32, 0.0, 0.05 * DEFAULT_DT as f32];
                world.prev.world_transforms[i].frame_delta_xyz =
                    world.curr.world_transforms[i].frame_delta_xyz;
            }
        }

        let speed = if move_speed > 0.0 {
            move_speed
        } else {
            DEFAULT_MOVE_SPEED
        };

        let snapshots = EpochSnapshotPair::new(world.prev.clone(), world.curr.clone());
        let scene_bounds = SceneBounds::default();
        let mvcc = match backend {
            SimBackendKind::EpochXpbd => None,
            SimBackendKind::MvccXpbd => {
                if world.curr.len() > ENTITIES_PER_PAGE {
                    return Err(format!(
                        "mvcc_xpbd requires sheet len ≤ {ENTITIES_PER_PAGE}, got {}",
                        world.curr.len()
                    ));
                }
                let mut adapter = PageXpbdAdapter::new(scene_bounds);
                adapter.set_gravity(world.gravity);
                adapter.set_substeps(world.substeps);
                Some(HostMvccState {
                    adapter,
                    page: HotPhysicsPage::zeroed_page(),
                })
            }
        };
        let mut host = Self {
            world,
            topo: FlatTopology::default(),
            snapshots,
            entity_rows,
            meshes,
            user_root_rows,
            user_root_spawns,
            user_root_quats,
            slot_count: slots,
            scales,
            boxes,
            colors,
            loco_mode,
            move_speed: speed,
            grabs: [None; USER_ROOT_SLOTS_MAX],
            grab_aims: [[0.0; 3]; USER_ROOT_SLOTS_MAX],
            slots: SlotGraph::new(),
            publish_dirty: true,
            ground_scratch: HashSet::new(),
            sanitize_scratch: HashSet::new(),
            active_row_scratch: Vec::new(),
            backend,
            scene_bounds,
            mvcc,
            barrier_ticks: 0,
            publishes: 0,
        };
        host.snapshots
            .publish(host.world.prev.clone(), host.world.curr.clone());
        host.publishes = 1;
        host.barrier_ticks = 1;
        Ok(host)
    }

    /// Export quantum-free sheet state for `…/sim_state` (x00b0). `drot` is always 0.
    pub fn export_sim_state(
        &self,
        room_id: &str,
        host_generation: u64,
        scene_rev: u64,
        sim_tick: u64,
        net_tick: u64,
    ) -> SimState {
        let mut holder_by_entity = HashMap::<u32, u32>::new();
        for slot in 0..self.slot_count {
            if let Some(ag) = self.grabs[slot] {
                let eid = self.boxes.entity_ids[ag.index];
                holder_by_entity.insert(eid, entity_holder_from_slot(slot as u8));
            }
        }
        let mut entities = Vec::with_capacity(self.entity_rows.len());
        let mut ids: Vec<u32> = self.entity_rows.keys().copied().collect();
        ids.sort_unstable();
        for id in ids {
            let Some(handle) = self.entity_rows.get(&id) else {
                continue;
            };
            let idx = handle.row_id as usize;
            let xf = &self.world.curr.world_transforms[idx];
            // Low 32 bits of sheet readiness (TOMBSTONE=1<<63 is not on the wire).
            // Non-zero → restored on import; `0` → keep Manifest/cold mask (crash snapshot).
            let flags = (self.world.curr.readiness[idx].0 & 0xffff_ffff) as u32;
            entities.push(SimEntity {
                id,
                x: xf.position[0],
                y: xf.position[1],
                z: xf.position[2],
                qx: xf.rotation[0],
                qy: xf.rotation[1],
                qz: xf.rotation[2],
                qw: xf.rotation[3],
                dx: xf.frame_delta_xyz[0],
                dy: xf.frame_delta_xyz[1],
                dz: xf.frame_delta_xyz[2],
                drot_x: 0.0,
                drot_y: 0.0,
                drot_z: 0.0,
                holder: holder_by_entity
                    .get(&id)
                    .copied()
                    .unwrap_or(ENTITY_HOLDER_NONE),
                flags,
            });
        }
        SimState {
            protocol_pin: PROTOCOL_PIN.into(),
            room_id: room_id.into(),
            host_generation,
            scene_rev,
            sim_tick,
            net_tick,
            entities,
            host_sig: Vec::new(),
        }
    }

    /// Resume from a quarantined [`SimState`] after Manifest topology seed (x00b0).
    ///
    /// Skips the cold-start mesh kick; applies poses/deltas/`flags`→readiness;
    /// restores grabs from `holder` (and re-asserts GRABBED).
    ///
    /// [`SimStateImportKind::Planned`] rejects all-zero `flags` (crash sentinel) so
    /// trajectory DoD cannot silently keep cold readiness.
    pub fn from_sim_state_with_backend(
        entries: &[SceneEntry],
        loco_mode: LocoMode,
        move_speed: f32,
        backend: SimBackendKind,
        state: &SimState,
        import: crate::sim_state::SimStateImportKind,
    ) -> Result<Self, String> {
        crate::sim_state::quarantine_sim_state(state, import).map_err(|e| e.to_string())?;
        let mut host = Self::from_entries_with_backend(entries, loco_mode, move_speed, backend)?;
        // Drop cold-start mesh kick before applying exported deltas.
        for i in 0..host.world.curr.len() {
            host.world.curr.world_transforms[i].frame_delta_xyz = [0.0, 0.0, 0.0];
            if i < host.world.prev.len() {
                host.world.prev.world_transforms[i].frame_delta_xyz = [0.0, 0.0, 0.0];
            }
        }
        host.apply_sim_state_entities(state, import)?;
        host.snapshots
            .publish(host.world.prev.clone(), host.world.curr.clone());
        host.publishes = host.publishes.saturating_add(1);
        host.publish_dirty = true;
        Ok(host)
    }

    fn apply_sim_state_entities(
        &mut self,
        state: &SimState,
        import: crate::sim_state::SimStateImportKind,
    ) -> Result<(), String> {
        let restore_readiness = import == crate::sim_state::SimStateImportKind::Planned;
        for e in &state.entities {
            let Some(handle) = self.entity_rows.get(&e.id).copied() else {
                continue;
            };
            let idx = handle.row_id as usize;
            let scale = self.world.curr.world_transforms[idx].scale;
            let inv_mass = self.world.curr.world_transforms[idx].inv_mass;
            let xf = WorldTransformf64 {
                position: [e.x, e.y, e.z],
                inv_mass,
                rotation: [e.qx, e.qy, e.qz, e.qw],
                scale,
                frame_delta_xyz: [e.dx, e.dy, e.dz],
                flags: 0,
                _pad0: 0,
                _pad1: [0; 4],
            };
            if !xf.is_finite() {
                return Err(format!(
                    "sim_state entity {} not finite after assemble",
                    e.id
                ));
            }
            let _ = self.world.curr.set_transform(handle.row_id, xf);
            // Planned: restore low-32 readiness (GRABBED re-applied below).
            // CrashPoseOnly: keep Manifest/cold readiness even if wire flags are non-zero.
            if restore_readiness {
                let mut mask = ReadinessMask(u64::from(e.flags));
                mask = mask.without_bits(ReadinessMask::GRABBED);
                self.world.curr.readiness[idx] = mask;
                if idx < self.world.prev.len() {
                    self.world.prev.readiness[idx] = mask;
                }
            }
            if idx < self.world.prev.len() {
                // Verlet prev ≈ curr − delta (epoch path restores via apply_frame_delta_to_prev).
                let mut prev = xf;
                prev.position = [
                    e.x - f64::from(e.dx),
                    e.y - f64::from(e.dy),
                    e.z - f64::from(e.dz),
                ];
                self.world.prev.world_transforms[idx] = prev;
            }
        }
        // Clear grabs then restore from holder (first holder wins on conflict).
        for slot in 0..self.slot_count {
            if self.grabs[slot].is_some() {
                self.release_grab(slot);
            }
        }
        // Drop any residual GRABBED from wire flags before holder restore.
        for e in &state.entities {
            if e.holder == ENTITY_HOLDER_NONE {
                self.set_grabbed_bit(e.id, false);
            }
        }
        for e in &state.entities {
            let Some(slot) = entity_holder_slot(e.holder) else {
                continue;
            };
            let slot = slot as usize;
            if slot >= self.slot_count || self.grabs[slot].is_some() {
                self.set_grabbed_bit(e.id, false);
                continue;
            }
            let Some(box_index) = self.boxes.entity_ids.iter().position(|&id| id == e.id) else {
                self.set_grabbed_bit(e.id, false);
                continue;
            };
            if !self
                .boxes
                .grabbable
                .get(box_index)
                .copied()
                .unwrap_or(false)
            {
                self.set_grabbed_bit(e.id, false);
                continue;
            }
            let (ux, uy, uz) = self.user_root_translation(slot);
            let (bx, by, bz) = (e.x as f32, e.y as f32, e.z as f32);
            let (ox, oy, oz) = crate::finite::clamp_hold_offset(bx - ux, by - uy, bz - uz);
            let holder_id = Self::user_root_entity_id(slot);
            if self.slots.attach_held(holder_id, e.id).is_err() {
                self.set_grabbed_bit(e.id, false);
                continue;
            }
            self.grabs[slot] = Some(ActiveGrab {
                index: box_index,
                offset: [ox, oy, oz],
            });
            self.grab_aims[slot] = [ux, uy, uz];
            self.set_grabbed_bit(e.id, true);
        }
        Ok(())
    }

    pub fn backend(&self) -> SimBackendKind {
        self.backend
    }

    #[allow(dead_code)]
    pub fn slot_graph(&self) -> &SlotGraph {
        &self.slots
    }

    #[allow(dead_code)]
    pub fn snapshots(&self) -> &EpochSnapshotPair {
        &self.snapshots
    }

    #[allow(dead_code)]
    pub fn entity_row(&self, entity_id: u32) -> Option<NodeHandle> {
        self.entity_rows.get(&entity_id).copied()
    }

    pub fn entity_count(&self) -> usize {
        self.entity_rows.len()
    }

    #[allow(dead_code)]
    pub fn pass_held(&mut self, from_slot: usize, to_slot: usize) -> bool {
        if from_slot >= self.slot_count || to_slot >= self.slot_count || from_slot == to_slot {
            return false;
        }
        let Some(g) = self.grabs[from_slot] else {
            return false;
        };
        if self.grabs[to_slot].is_some() {
            return false;
        }
        let (fx, fy, fz) = self.user_root_translation(from_slot);
        let (tx, ty, tz) = self.user_root_translation(to_slot);
        if !crate::finite::within_grab_start_distance((fx, fy, fz), (tx, ty, tz)) {
            return false;
        }
        let held_id = self.boxes.entity_ids[g.index];
        let holder_to = Self::user_root_entity_id(to_slot);
        let (bx, by, bz) = self.box_pose(g.index).0;
        let raw = [bx - tx, by - ty, bz - tz];
        let (ox, oy, oz) = crate::finite::clamp_hold_offset(raw[0], raw[1], raw[2]);
        let new_g = ActiveGrab {
            index: g.index,
            offset: [ox, oy, oz],
        };
        if self.slots.attach_held(holder_to, held_id).is_err() {
            return false;
        }
        self.grabs[from_slot] = None;
        self.grab_aims[to_slot] = [tx, ty, tz];
        self.grabs[to_slot] = Some(new_g);
        self.set_grabbed_bit(held_id, true);
        true
    }

    #[allow(dead_code)]
    pub fn place_held(&mut self, slot: usize) -> bool {
        if slot >= self.slot_count || self.grabs[slot].is_none() {
            return false;
        }
        self.release_grab(slot);
        true
    }

    pub fn user_root_slot_count(&self) -> usize {
        self.slot_count
    }

    pub fn mesh_entity_id(&self) -> Option<u32> {
        self.meshes.first().copied()
    }

    pub fn mesh_entity_ids(&self) -> impl Iterator<Item = u32> + '_ {
        self.meshes.iter().copied()
    }

    pub fn despawn_mesh_entity(&mut self, entity_id: u32) -> bool {
        let Some(pos) = self.meshes.iter().position(|&id| id == entity_id) else {
            return false;
        };
        let Some(handle) = self.entity_rows.remove(&entity_id) else {
            self.meshes.remove(pos);
            self.colors.remove(&entity_id);
            self.scales.remove(&entity_id);
            return false;
        };
        if self.world.curr.tombstone(handle).is_err() {
            self.entity_rows.insert(entity_id, handle);
            return false;
        }
        self.meshes.remove(pos);
        self.colors.remove(&entity_id);
        self.scales.remove(&entity_id);
        xpbd::sync_prev_capacity(&mut self.world);
        self.publish_dirty = true;
        true
    }

    pub fn spawn_mesh_entry(&mut self, entry: &SceneEntry) -> bool {
        if classify_scene_entry(entry) != EntityKind::Mesh {
            return false;
        }
        if self.meshes.contains(&entry.entity_id)
            || self.boxes.entity_ids.contains(&entry.entity_id)
            || self.entity_rows.contains_key(&entry.entity_id)
        {
            return false;
        }
        let handle = alloc_dynamic_sphere(
            &mut self.world,
            [entry.x, entry.y, entry.z],
            entry_quat_xyzw(entry),
            entry_scale_f32(entry),
            entry.entity_id,
            PHYS_BALL_RADIUS,
            true,
        );
        xpbd::sync_prev_capacity(&mut self.world);
        let i = handle.row_id as usize;
        if i < self.world.prev.len() {
            self.world.prev.world_transforms[i] = self.world.curr.world_transforms[i];
        }
        self.world.curr.world_transforms[i].frame_delta_xyz =
            [0.05 * DEFAULT_DT as f32, 0.0, 0.02 * DEFAULT_DT as f32];
        self.colors
            .insert(entry.entity_id, scene_entry_color_rgba(entry));
        self.scales.insert(entry.entity_id, entry_scale_f32(entry));
        self.entity_rows.insert(entry.entity_id, handle);
        self.meshes.push(entry.entity_id);
        self.publish_dirty = true;
        true
    }

    pub fn mesh_pose_for(&self, entity_id: u32) -> Option<((f32, f32, f32), [f32; 4])> {
        if !self.meshes.contains(&entity_id) {
            return None;
        }
        let handle = self.entity_rows.get(&entity_id)?;
        Some(self.pose_from_row(handle.row_id))
    }

    pub fn color_for(&self, entity_id: u32) -> (f32, f32, f32, f32) {
        self.colors
            .get(&entity_id)
            .copied()
            .unwrap_or((1.0, 1.0, 1.0, 1.0))
    }

    pub fn scale_for(&self, entity_id: u32) -> [f32; 3] {
        self.scales
            .get(&entity_id)
            .copied()
            .unwrap_or([1.0, 1.0, 1.0])
    }

    pub fn user_root_entity_id(slot: usize) -> u32 {
        USER_ROOT_ENTITY_FIRST + slot as u32
    }

    fn entity_index(&self, entity_id: u32) -> Option<usize> {
        self.boxes
            .entity_ids
            .iter()
            .position(|&id| id == entity_id)
            .filter(|&i| self.boxes.grabbable.get(i).copied().unwrap_or(false))
    }

    fn box_held_by_other(&self, slot: usize, box_index: usize) -> bool {
        self.grabs[..self.slot_count]
            .iter()
            .enumerate()
            .any(|(s, g)| s != slot && g.map(|ag| ag.index == box_index).unwrap_or(false))
    }

    pub fn apply_grab(
        &mut self,
        slot: usize,
        want_grab: bool,
        entity_id: u32,
        aim: (f32, f32, f32),
    ) {
        if slot >= self.user_root_slot_count() {
            return;
        }
        if !want_grab {
            self.release_grab(slot);
            return;
        }
        let (ux, uy, uz) = self.user_root_translation(slot);
        let (ax, ay, az) = crate::finite::clamp_hold_aim((ux, uy, uz), aim);
        self.grab_aims[slot] = [ax, ay, az];
        if self.grabs[slot].is_some() {
            return;
        }
        let Some(index) = self.entity_index(entity_id) else {
            return;
        };
        if self.box_held_by_other(slot, index) {
            return;
        }
        let (tx, ty, tz) = self.box_pose(index).0;
        if !crate::finite::within_grab_start_distance((ux, uy, uz), (tx, ty, tz)) {
            return;
        }
        let raw = [tx - ax, ty - ay, tz - az];
        let (ox, oy, oz) = crate::finite::clamp_hold_offset(raw[0], raw[1], raw[2]);
        let held_id = self.boxes.entity_ids[index];
        let holder = Self::user_root_entity_id(slot);
        if self.slots.attach_held(holder, held_id).is_err() {
            return;
        }
        self.grabs[slot] = Some(ActiveGrab {
            index,
            offset: [ox, oy, oz],
        });
        self.set_grabbed_bit(held_id, true);
    }

    fn set_grabbed_bit(&mut self, held_id: u32, grabbed: bool) {
        let Some(handle) = self.entity_rows.get(&held_id).copied() else {
            return;
        };
        let idx = handle.row_id as usize;
        let mask = self.world.curr.readiness[idx];
        self.world.curr.readiness[idx] = if grabbed {
            mask.with_bits(ReadinessMask::GRABBED)
                .without_bits(ReadinessMask::SLEEPING)
        } else {
            mask.without_bits(ReadinessMask::GRABBED)
        };
    }

    fn release_grab(&mut self, slot: usize) {
        let Some(g) = self.grabs[slot].take() else {
            return;
        };
        let held_id = self.boxes.entity_ids[g.index];
        self.slots.clear_held(held_id);
        self.set_grabbed_bit(held_id, false);
        if let Some(handle) = self.entity_rows.get(&held_id).copied() {
            let idx = handle.row_id as usize;
            // Drop from rest (no throw impulse — non-guarantee).
            self.world.curr.world_transforms[idx].frame_delta_xyz = [0.0, 0.0, 0.0];
        }
        // Align with Intent/spawn/`release_slot`: mask/delta change is outside XPBD displace.
        self.publish_dirty = true;
    }

    pub fn release_slot(&mut self, slot: usize) {
        if slot >= self.user_root_slot_count() {
            return;
        }
        self.release_grab(slot);
        let (sx, sy, sz) = self.user_root_spawns[slot];
        let rot = self.user_root_quats[slot];
        let Some(handle) = self.user_root_rows[slot] else {
            return;
        };
        let scale = self.scale_for(Self::user_root_entity_id(slot));
        let xf = xf_at([sx as f64, sy as f64, sz as f64], rot, scale, 0.0);
        let _ = self.world.curr.set_transform(handle.row_id, xf);
        let idx = handle.row_id as usize;
        if idx < self.world.prev.len() {
            self.world.prev.world_transforms[idx] = xf;
        }
        self.publish_dirty = true;
    }

    pub fn grab_entity_id(&self, slot: usize) -> Option<u32> {
        if slot >= self.slot_count {
            return None;
        }
        self.grabs[slot].map(|ag| self.boxes.entity_ids[ag.index])
    }

    /// Box SoA index held by `slot`, if any (wire holder fill · x00af).
    pub fn grabbed_box_index(&self, slot: usize) -> Option<usize> {
        if slot >= self.slot_count {
            return None;
        }
        self.grabs[slot].map(|ag| ag.index)
    }

    pub fn loco_mode(&self) -> LocoMode {
        self.loco_mode
    }

    /// One fixed 60Hz XPBD tick + Prev/Curr publish (unit tests; production uses [`Self::step_dt`]).
    #[cfg(test)]
    pub fn step(&mut self) {
        self.step_dt(DEFAULT_DT as f32);
    }

    pub fn step_dt(&mut self, dt: f32) {
        let dt = if dt.is_finite() && dt > 0.0 {
            f64::from(dt)
        } else {
            DEFAULT_DT
        };

        let grab_targets = self.collect_grab_targets();
        xpbd::sync_prev_capacity(&mut self.world);
        let start = self.world.curr.clone();
        match self.backend {
            SimBackendKind::EpochXpbd => {
                // Host barrier: restore Verlet prev from frame_delta (epoch path only).
                // mvcc_xpbd rebuilds Prev inside PageXpbdAdapter::pull_page — do not dual-apply.
                self.apply_frame_delta_to_prev();
                // x00ad: body-body LBVH contacts on (x0093 had bounds: None).
                xpbd::step(
                    &mut self.world,
                    &mut self.topo,
                    &[],
                    &grab_targets,
                    Some(self.scene_bounds),
                    dt,
                );
            }
            SimBackendKind::MvccXpbd => {
                self.step_mvcc_page(&grab_targets, dt);
            }
        }
        self.resolve_ground_plane();
        self.sanitize_non_finite_dynamics();
        // Re-run sleep against tick-start pose (step's update_sleeping saw Verlet prev).
        self.copy_positions_into_prev(&start);
        xpbd::update_sleeping(&mut self.world);
        self.write_frame_deltas_from(&start);
        // Include `publish_dirty`: Intent/kinematic edits happen before `step_dt`, so
        // start==curr would otherwise skip and leave snapshot consumers on a stale face.
        let moved = self.publish_dirty
            || self.poses_differ(&start)
            || !self.ground_scratch.is_empty()
            || !self.sanitize_scratch.is_empty()
            || self.has_active_grab();
        if moved {
            self.snapshots.publish(start, self.world.curr.clone());
            self.publishes = self.publishes.wrapping_add(1);
            self.publish_dirty = false;
        }
        self.world.curr.clear_dirty();
        self.world.commit_prev_from_curr();
        self.barrier_ticks = self.barrier_ticks.wrapping_add(1);
    }

    /// Sync Epoch → page, XPBD+contacts+grabs on page, copy poses back (x00ad bridge).
    fn step_mvcc_page(&mut self, grab_targets: &[(u32, [f64; 3])], dt: f64) {
        let sub = self.world.substeps.max(1) as f32;
        let inv_sub = 1.0 / sub;
        // `grabs[]` is SSoT for page holder (not sheet GRABBED bit alone).
        let mut holders = [NO_HOLDER; ENTITIES_PER_PAGE];
        for slot in 0..self.slot_count {
            let Some(g) = self.grabs[slot] else {
                continue;
            };
            let held_id = self.boxes.entity_ids[g.index];
            let Some(handle) = self.entity_rows.get(&held_id) else {
                continue;
            };
            let row = handle.row_id as usize;
            if row < ENTITIES_PER_PAGE {
                holders[row] = slot as u8;
            }
        }
        let mvcc = self
            .mvcc
            .as_mut()
            .expect("mvcc_xpbd requires HostMvccState (constructed in from_entries_with_backend)");
        mvcc.adapter.bounds = self.scene_bounds;
        mvcc.adapter.set_gravity(self.world.gravity);
        mvcc.adapter.set_substeps(self.world.substeps);
        sync_sheet_to_page(
            &self.world.curr,
            &mut mvcc.page,
            &mut mvcc.adapter,
            inv_sub,
            &holders,
        );
        mvcc.adapter
            .step_page_with_grabs(&mut mvcc.page, grab_targets, dt);
        sync_page_poses_to_sheet(&mvcc.page, &mut self.world.curr);
    }

    /// Test-only: quarantine check on the MVCC page face.
    ///
    /// **Not** production Ingress — the next [`Self::step_dt`] re-syncs from Epoch and
    /// overwrites page writes. Page-first Intent lands with dirty / `sim_state` work.
    #[cfg(test)]
    pub fn try_ingest_mvcc_kinematic_for_test(
        &mut self,
        local_idx: usize,
        pose: KinematicPoseF32,
    ) -> bool {
        let Some(mvcc) = self.mvcc.as_mut() else {
            return false;
        };
        mvcc.page.try_set_kinematic_pose(local_idx, pose)
    }

    #[cfg(test)]
    pub fn mvcc_page(&self) -> Option<&HotPhysicsPage> {
        self.mvcc.as_ref().map(|m| &m.page)
    }

    /// Restore Verlet `prev` from last tick's **frame** displacement.
    ///
    /// `xpbd::step` runs `substeps` Verlet iterations; each uses `(curr - sub_prev)` as the
    /// *substep* step. `frame_delta_xyz` is the full-frame displacement (`end - start`), so we
    /// seed `prev = curr - delta / substeps`. Seeding `curr - delta` makes every substep re-add
    /// the full frame step → exact `× substeps` velocity blow-up per tick (coin → Inf, Frames
    /// dropped, loco appears frozen).
    fn apply_frame_delta_to_prev(&mut self) {
        let n = self.world.curr.len().min(self.world.prev.len());
        let sub = self.world.substeps.max(1) as f32;
        let inv_sub = 1.0 / sub;
        for i in 0..n {
            let d = self.world.curr.world_transforms[i].frame_delta_xyz;
            let c = self.world.curr.world_transforms[i].position;
            self.world.prev.world_transforms[i].position = [
                c[0] - (d[0] * inv_sub) as f64,
                c[1] - (d[1] * inv_sub) as f64,
                c[2] - (d[2] * inv_sub) as f64,
            ];
        }
    }

    fn copy_positions_into_prev(&mut self, src: &alchemy_shared_sim::EpochSheet) {
        let n = self.world.prev.len().min(src.len());
        for i in 0..n {
            self.world.prev.world_transforms[i].position = src.world_transforms[i].position;
        }
    }

    fn has_active_grab(&self) -> bool {
        self.grabs[..self.slot_count].iter().any(|g| g.is_some())
    }

    fn poses_differ(&self, start: &alchemy_shared_sim::EpochSheet) -> bool {
        let n = self.world.curr.len().min(start.len());
        for i in 0..n {
            let a = &start.world_transforms[i].position;
            let b = &self.world.curr.world_transforms[i].position;
            let dx = a[0] - b[0];
            let dy = a[1] - b[1];
            let dz = a[2] - b[2];
            if dx * dx + dy * dy + dz * dz > 1e-20 {
                return true;
            }
            if start.world_transforms[i].rotation != self.world.curr.world_transforms[i].rotation {
                return true;
            }
        }
        false
    }

    /// Write Verlet handoff deltas. Grounded rows get **vertical delta = 0** (instant kill).
    /// Sanitized rows get **full delta = 0** (must not use `end(origin) - start(pre-explode)`).
    fn write_frame_deltas_from(&mut self, start: &alchemy_shared_sim::EpochSheet) {
        let n = self.world.curr.len().min(start.len());
        for i in 0..n {
            let mask = self.world.curr.readiness[i];
            if mask.contains(ReadinessMask::TOMBSTONE)
                || !mask.is_ready(ReadinessMask::DYNAMIC)
                || mask.contains(ReadinessMask::GRABBED)
                || self.world.curr.world_transforms[i].inv_mass == 0.0
                || self.sanitize_scratch.contains(&(i as u32))
            {
                self.world.curr.world_transforms[i].frame_delta_xyz = [0.0, 0.0, 0.0];
                continue;
            }
            let s = start.world_transforms[i].position;
            let e = self.world.curr.world_transforms[i].position;
            let mut dy = (e[1] - s[1]) as f32;
            if self.ground_scratch.contains(&(i as u32)) {
                dy = 0.0;
            }
            self.world.curr.world_transforms[i].frame_delta_xyz =
                [(e[0] - s[0]) as f32, dy, (e[2] - s[2]) as f32];
        }
    }

    fn collect_grab_targets(&self) -> Vec<(u32, [f64; 3])> {
        let mut out = Vec::new();
        for slot in 0..self.slot_count {
            let Some(g) = self.grabs[slot] else {
                continue;
            };
            let held_id = self.boxes.entity_ids[g.index];
            let Some(handle) = self.entity_rows.get(&held_id) else {
                continue;
            };
            let aim = self.grab_aims[slot];
            out.push((
                handle.row_id,
                [
                    (aim[0] + g.offset[0]) as f64,
                    (aim[1] + g.offset[1]) as f64,
                    (aim[2] + g.offset[2]) as f64,
                ],
            ));
        }
        out
    }

    /// Freeze non-finite DYNAMIC rows at ground support.
    ///
    /// Marks [`Self::sanitize_scratch`] so [`Self::write_frame_deltas_from`] keeps delta at 0
    /// (otherwise `end(origin) - start(pre-explode)` re-fuels the next tick).
    fn sanitize_non_finite_dynamics(&mut self) {
        self.sanitize_scratch.clear();
        self.active_row_scratch.clear();
        self.active_row_scratch
            .extend(self.world.curr.iter_active_rows());
        for &row in &self.active_row_scratch {
            let idx = row as usize;
            let mask = self.world.curr.readiness[idx];
            if !mask.is_ready(ReadinessMask::DYNAMIC) || mask.contains(ReadinessMask::GRABBED) {
                continue;
            }
            let xf = &self.world.curr.world_transforms[idx];
            if xf.is_finite() {
                continue;
            }
            let entity_id = self.world.curr.owner_scope[idx];
            log::warn!(
                "sanitize non-finite DYNAMIC row={row} entity_id={entity_id} pos={:?} delta={:?}",
                xf.position,
                xf.frame_delta_xyz
            );
            let support = support_half_y(&self.world.curr.colliders[idx]);
            let mut fixed = *xf;
            fixed.position = [0.0, GROUND_Y + support, 0.0];
            fixed.frame_delta_xyz = [0.0, 0.0, 0.0];
            if !fixed.rotation.iter().all(|v| v.is_finite()) {
                fixed.rotation = [0.0, 0.0, 0.0, 1.0];
            }
            if !fixed.scale.iter().all(|v| v.is_finite()) || fixed.scale.contains(&0.0) {
                fixed.scale = [1.0, 1.0, 1.0];
            }
            let _ = self.world.curr.set_transform(row, fixed);
            self.sanitize_scratch.insert(row);
            self.publish_dirty = true;
        }
    }

    /// Fills [`Self::ground_scratch`] with rows that contacted the ground this tick
    /// (vertical `frame_delta` → 0 in [`Self::write_frame_deltas_from`]).
    fn resolve_ground_plane(&mut self) {
        self.ground_scratch.clear();
        self.active_row_scratch.clear();
        self.active_row_scratch
            .extend(self.world.curr.iter_active_rows());
        for &row in &self.active_row_scratch {
            let idx = row as usize;
            let mask = self.world.curr.readiness[idx];
            if !mask.is_ready(ReadinessMask::DYNAMIC) || mask.contains(ReadinessMask::GRABBED) {
                continue;
            }
            if self.world.curr.world_transforms[idx].inv_mass == 0.0 {
                continue;
            }
            let support = support_half_y(&self.world.curr.colliders[idx]);
            let min_y = GROUND_Y + support;
            let y = self.world.curr.world_transforms[idx].position[1];
            if y < min_y {
                self.world.curr.world_transforms[idx].position[1] = min_y;
                self.ground_scratch.insert(row);
                self.world.curr.mark_dirty(row);
            }
        }
    }

    pub fn apply_user_intent(&mut self, slot: usize, mx: f32, my: f32, mz: f32, dt: f32) {
        if slot >= self.user_root_slot_count() {
            return;
        }
        let (mx, my, mz) = crate::finite::clamp_move_vector(mx, my, mz);
        let speed = self.move_speed;
        let Some(handle) = self.user_root_rows[slot] else {
            return;
        };
        let idx = handle.row_id as usize;
        let mut xf = self.world.curr.world_transforms[idx];
        let t = xf.position;
        match self.loco_mode {
            LocoMode::Noclip | LocoMode::Fly => {
                xf.position = [
                    t[0] + (mx * speed * dt) as f64,
                    t[1] + (my * speed * dt) as f64,
                    t[2] + (mz * speed * dt) as f64,
                ];
            }
            LocoMode::Walk => {
                xf.position = [
                    t[0] + (mx * speed * dt) as f64,
                    t[1],
                    t[2] + (mz * speed * dt) as f64,
                ];
            }
        }
        // Neutral stick (`overlay.active` stays true) must not defeat idle publish skip.
        let moved = xf.position != t;
        if moved && xf.is_finite() {
            let _ = self.world.curr.set_transform(handle.row_id, xf);
            if idx < self.world.prev.len() {
                self.world.prev.world_transforms[idx] = xf;
            }
            self.publish_dirty = true;
        }
    }

    /// Test helper: teleport a user_root without Intent (pass_held distance setup).
    #[cfg(test)]
    pub fn teleport_user_root(&mut self, slot: usize, t: (f32, f32, f32)) {
        if slot >= self.slot_count {
            return;
        }
        let Some(handle) = self.user_root_rows[slot] else {
            return;
        };
        let idx = handle.row_id as usize;
        let mut xf = self.world.curr.world_transforms[idx];
        xf.position = [t.0 as f64, t.1 as f64, t.2 as f64];
        let _ = self.world.curr.set_transform(handle.row_id, xf);
        if idx < self.world.prev.len() {
            self.world.prev.world_transforms[idx] = xf;
        }
        self.publish_dirty = true;
    }

    pub fn moving_pose(&self) -> ((f32, f32, f32), [f32; 4]) {
        let Some(id) = self.meshes.first().copied() else {
            return ((0.0, 5.0, 0.0), [0.0, 0.0, 0.0, 1.0]);
        };
        self.mesh_pose_for(id)
            .unwrap_or(((0.0, 5.0, 0.0), [0.0, 0.0, 0.0, 1.0]))
    }

    pub fn user_root_pose(&self, slot: usize) -> ((f32, f32, f32), [f32; 4]) {
        let fallback = (
            if slot < USER_ROOT_SLOTS_MAX {
                self.user_root_spawns[slot]
            } else {
                (0.0, 1.6, 6.0)
            },
            [0.0, 0.0, 0.0, 1.0],
        );
        if slot >= self.slot_count {
            return fallback;
        }
        self.user_root_rows[slot]
            .map(|h| self.pose_from_row(h.row_id))
            .unwrap_or(fallback)
    }

    pub fn box_pose(&self, index: usize) -> ((f32, f32, f32), [f32; 4]) {
        let he = self.boxes.half_extents.get(index).copied().unwrap_or(0.25);
        let Some(&id) = self.boxes.entity_ids.get(index) else {
            return ((0.0, he, 0.0), [0.0, 0.0, 0.0, 1.0]);
        };
        self.entity_rows
            .get(&id)
            .map(|h| self.pose_from_row(h.row_id))
            .unwrap_or(((0.0, he, 0.0), [0.0, 0.0, 0.0, 1.0]))
    }

    fn pose_from_row(&self, row_id: u32) -> ((f32, f32, f32), [f32; 4]) {
        let idx = row_id as usize;
        let xf = &self.world.curr.world_transforms[idx];
        (
            (
                xf.position[0] as f32,
                xf.position[1] as f32,
                xf.position[2] as f32,
            ),
            xf.rotation,
        )
    }

    pub fn moving_translation(&self) -> (f32, f32, f32) {
        self.moving_pose().0
    }

    pub fn user_root_translation(&self, slot: usize) -> (f32, f32, f32) {
        self.user_root_pose(slot).0
    }
}

fn alloc_dynamic_sphere(
    world: &mut XpbdWorld,
    pos: [f64; 3],
    rot: [f32; 4],
    scale: [f32; 3],
    entity_id: u32,
    radius: f32,
    mesh: bool,
) -> NodeHandle {
    let handle = world.curr.alloc_row();
    let xf = xf_at(pos, rot, scale, 1.0);
    let _ = world.curr.set_transform(handle.row_id, xf);
    world.curr.colliders[handle.row_id as usize] = ColliderShape {
        shape_type: ColliderShape::SPHERE,
        extents: [radius, 0.0, 0.0],
    };
    let mut mask = ReadinessMask(
        ReadinessMask::ALLOCATED
            | ReadinessMask::TRANSFORM
            | ReadinessMask::COLLIDER
            | ReadinessMask::DYNAMIC,
    );
    if mesh {
        mask = mask.with_bits(ReadinessMask::MESH);
    }
    world.curr.readiness[handle.row_id as usize] = mask;
    world.curr.owner_scope[handle.row_id as usize] = entity_id;
    handle
}

/// Distinct tint per user_root slot when Manifest omits color.
pub fn user_root_color(slot: usize) -> (f32, f32, f32, f32) {
    // DemoScene ships four roots; further slots cycle (Manifest may go up to MAX).
    const PALETTE: [(f32, f32, f32); 4] = [
        (0.4, 0.7, 1.0),
        (1.0, 0.55, 0.25),
        (0.45, 0.9, 0.5),
        (0.9, 0.45, 0.85),
    ];
    let (r, g, b) = PALETTE[slot % PALETTE.len()];
    (r, g, b, 1.0)
}

/// Copy live Epoch rows into a HotPhysicsPage for [`PageXpbdAdapter`].
///
/// `delta_scale` is typically `1 / substeps` so page Prev rebuild matches the
/// epoch handoff (`curr - delta/substeps`) and avoids ×substeps velocity blow-up.
///
/// `holder_by_row[local]` is the Grab owner slot (`NO_HOLDER` if free) — must match
/// host `grabs[]` (not hard-coded 0) so later `Entity.holder` wire stays honest.
///
/// **Near-origin only (x00ad):** `cell_origin` is forced to 0; world f64 is cast to
/// local f32. Far-from-origin scenes need Directory cell origins (post–dirty).
fn sync_sheet_to_page(
    sheet: &EpochSheet,
    page: &mut HotPhysicsPage,
    adapter: &mut PageXpbdAdapter,
    delta_scale: f32,
    holder_by_row: &[u8; ENTITIES_PER_PAGE],
) {
    page.cell_origin_x = 0.0;
    page.cell_origin_y = 0.0;
    page.cell_origin_z = 0.0;
    let n = sheet.len().min(ENTITIES_PER_PAGE);
    for (idx, &holder) in holder_by_row.iter().enumerate() {
        if idx >= n {
            page.set_active(idx, false);
            page.clear_holder(idx);
            continue;
        }
        let mask = sheet.readiness[idx];
        if mask.contains(ReadinessMask::TOMBSTONE) || !mask.contains(ReadinessMask::ALLOCATED) {
            page.set_active(idx, false);
            page.clear_holder(idx);
            continue;
        }
        let xf = &sheet.world_transforms[idx];
        if !xf.is_finite() {
            page.quarantine_slot(idx);
            continue;
        }
        let lx = xf.position[0] as f32;
        let ly = xf.position[1] as f32;
        let lz = xf.position[2] as f32;
        let dx = xf.frame_delta_xyz[0] * delta_scale;
        let dy = xf.frame_delta_xyz[1] * delta_scale;
        let dz = xf.frame_delta_xyz[2] * delta_scale;
        if !(lx.is_finite()
            && ly.is_finite()
            && lz.is_finite()
            && dx.is_finite()
            && dy.is_finite()
            && dz.is_finite())
        {
            page.quarantine_slot(idx);
            continue;
        }
        page.set_active(idx, true);
        page.soa.pos_x[idx] = lx;
        page.soa.pos_y[idx] = ly;
        page.soa.pos_z[idx] = lz;
        page.soa.delta_x[idx] = dx;
        page.soa.delta_y[idx] = dy;
        page.soa.delta_z[idx] = dz;
        page.soa.inv_mass[idx] = xf.inv_mass as f32;
        page.soa.rot_x[idx] = xf.rotation[0];
        page.soa.rot_y[idx] = xf.rotation[1];
        page.soa.rot_z[idx] = xf.rotation[2];
        page.soa.rot_w[idx] = xf.rotation[3];
        page.set_sleeping(idx, mask.contains(ReadinessMask::SLEEPING));
        if holder != NO_HOLDER {
            let _ = page.set_holder(idx, holder, 0);
        } else {
            page.clear_holder(idx);
        }
        adapter.colliders.set(idx, sheet.colliders[idx]);
    }
}

/// Write page local poses back into Epoch Curr (f64 = cell_origin + local).
fn sync_page_poses_to_sheet(page: &HotPhysicsPage, sheet: &mut EpochSheet) {
    let ox = page.cell_origin_x;
    let oy = page.cell_origin_y;
    let oz = page.cell_origin_z;
    let n = sheet.len().min(ENTITIES_PER_PAGE);
    for idx in 0..n {
        if !page.is_active(idx) {
            continue;
        }
        let lx = page.soa.pos_x[idx];
        let ly = page.soa.pos_y[idx];
        let lz = page.soa.pos_z[idx];
        if !(lx.is_finite() && ly.is_finite() && lz.is_finite()) {
            continue;
        }
        let xf = &mut sheet.world_transforms[idx];
        xf.position = [ox + f64::from(lx), oy + f64::from(ly), oz + f64::from(lz)];
        xf.rotation = [
            page.soa.rot_x[idx],
            page.soa.rot_y[idx],
            page.soa.rot_z[idx],
            page.soa.rot_w[idx],
        ];
        // Poses only for all rows. SLEEPING write-back is DYNAMIC-only — kinematic
        // user_roots must never pick up SLEEPING from the page adapter face.
        let mut mask = sheet.readiness[idx];
        if mask.is_ready(ReadinessMask::DYNAMIC) {
            if page.is_sleeping(idx) {
                mask = mask.with_bits(ReadinessMask::SLEEPING);
            } else {
                mask = mask.without_bits(ReadinessMask::SLEEPING);
            }
            sheet.readiness[idx] = mask;
        }
    }
}

/// Unit-test fixture mirroring `World.DemoScene` layout (x0079 shared vector).
#[cfg(test)]
pub fn demo_entries_for_test() -> Vec<SceneEntry> {
    alchemy_shared_sim::demo::demo_scene_entries()
}

#[cfg(test)]
fn test_world() -> PhysicsWorld {
    PhysicsWorld::from_entries(
        &demo_entries_for_test(),
        LocoMode::Noclip,
        DEFAULT_MOVE_SPEED,
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sim_state::SimStateImportKind;

    #[test]
    fn ball_falls_onto_floor() {
        let mut world = PhysicsWorld::from_entries(
            &demo_entries_for_test(),
            LocoMode::Walk,
            DEFAULT_MOVE_SPEED,
        );
        let (x0, y0, z0) = world.moving_translation();
        assert!(y0 > 4.5, "spawn above floor");
        for _ in 0..600 {
            world.step();
        }
        let (x, y, z) = world.moving_translation();
        assert!(
            x.is_finite() && y.is_finite() && z.is_finite(),
            "mesh must stay finite, got ({x},{y},{z}) from ({x0},{y0},{z0})"
        );
        assert!(y > 0.1 && y < 1.0, "expected resting on floor, got y={y}");
        assert!(
            x.abs() < 50.0 && z.abs() < 50.0,
            "horizontal must not explode, got ({x},{z})"
        );
    }

    #[test]
    fn ground_contact_tick_zeros_vertical_frame_delta() {
        let mut world = PhysicsWorld::from_entries(
            &demo_entries_for_test(),
            LocoMode::Walk,
            DEFAULT_MOVE_SPEED,
        );
        let mesh_id = world.mesh_entity_id().unwrap();
        let row = world.entity_row(mesh_id).unwrap().row_id as usize;
        let min_y = GROUND_Y + PHYS_BALL_RADIUS as f64;
        // Place just above floor with strong downward Verlet so the next tick clamps.
        {
            let xf = &mut world.world.curr.world_transforms[row];
            xf.position = [0.0, min_y + 0.02, 0.0];
            xf.frame_delta_xyz = [0.0, -1.0, 0.0];
            let mask = world.world.curr.readiness[row];
            world.world.curr.readiness[row] = mask.without_bits(ReadinessMask::SLEEPING);
        }
        let y_before = world.world.curr.world_transforms[row].position[1];
        assert!(y_before > min_y, "fixture must start above ground");
        world.step();
        let y_after = world.world.curr.world_transforms[row].position[1];
        let dy = world.world.curr.world_transforms[row].frame_delta_xyz[1];
        assert!(
            (y_after - min_y).abs() < 1e-9,
            "contact tick must clamp to ground, y_after={y_after} min_y={min_y}"
        );
        assert!(
            world.ground_scratch.contains(&(row as u32)),
            "contact tick must mark row grounded"
        );
        assert!(
            dy.abs() <= 1e-6,
            "contact tick vertical frame_delta must be 0 (±eps), got {dy}"
        );
    }

    #[test]
    fn intent_forces_snapshot_publish_when_poses_unchanged_by_xpbd() {
        let mut world =
            PhysicsWorld::from_entries(&demo_entries_for_test(), LocoMode::Noclip, 10.0);
        // Settle so idle ticks would skip publish.
        for _ in 0..30 {
            world.step();
        }
        let pubs = world.publishes;
        world.publish_dirty = false;
        world.apply_user_intent(0, 1.0, 0.0, 0.0, 1.0 / 60.0);
        world.step();
        assert!(
            world.publishes > pubs,
            "Intent kinematic move must publish snapshots for consumers"
        );
        let snap = world.snapshots().load_curr();
        let id = PhysicsWorld::user_root_entity_id(0);
        let row = world.entity_row(id).unwrap().row_id as usize;
        let live = world.world.curr.world_transforms[row].position;
        let published = snap.world_transforms[row].position;
        assert_eq!(
            live, published,
            "snapshot Curr must match live after Intent"
        );
    }

    #[test]
    fn neutral_intent_does_not_set_publish_dirty() {
        let mut world =
            PhysicsWorld::from_entries(&demo_entries_for_test(), LocoMode::Noclip, 10.0);
        for _ in 0..10 {
            world.step();
        }
        world.publish_dirty = false;
        world.apply_user_intent(0, 0.0, 0.0, 0.0, 1.0 / 60.0);
        assert!(
            !world.publish_dirty,
            "zero-displacement Intent must leave idle publish skip intact"
        );
    }

    #[test]
    fn release_grab_and_place_held_set_publish_dirty() {
        let mut world = test_world();
        for _ in 0..60 {
            world.step();
        }
        let id = 14u32;
        let (bx, by, bz) = world.box_pose(4).0;
        world.apply_grab(0, true, id, (bx, by, bz));
        assert_eq!(world.grab_entity_id(0), Some(id));
        world.publish_dirty = false;
        world.apply_grab(0, false, 0, (bx, by, bz));
        assert!(world.publish_dirty, "release_grab must set publish_dirty");

        world.apply_grab(0, true, id, (bx, by, bz));
        world.publish_dirty = false;
        assert!(world.place_held(0));
        assert!(
            world.publish_dirty,
            "place_held → release_grab must set publish_dirty"
        );
    }

    #[test]
    fn noclip_passes_through_floor() {
        let mut world =
            PhysicsWorld::from_entries(&demo_entries_for_test(), LocoMode::Noclip, 10.0);
        let dt = 1.0 / 60.0;
        for _ in 0..120 {
            world.apply_user_intent(0, 0.0, -1.0, 0.0, dt);
            world.step();
        }
        let (_, y, _) = world.user_root_translation(0);
        assert!(y < 0.0, "noclip should pass below floor, got y={y}");
    }

    #[test]
    fn two_slots_move_independently() {
        let mut world =
            PhysicsWorld::from_entries(&demo_entries_for_test(), LocoMode::Noclip, 10.0);
        let dt = 1.0 / 60.0;
        for _ in 0..60 {
            world.apply_user_intent(0, 1.0, 0.0, 0.0, dt);
            world.apply_user_intent(1, -1.0, 0.0, 0.0, dt);
            world.step();
        }
        let (x0, _, _) = world.user_root_translation(0);
        let (x1, _, _) = world.user_root_translation(1);
        assert!(x0 > 0.5, "slot0 should move +X, got {x0}");
        assert!(x1 < 1.0, "slot1 should move -X from 1.5, got {x1}");
        assert!(x0 > x1, "slot0 ahead of slot1 on X");
    }

    #[test]
    fn four_slots_have_distinct_spawns() {
        let world = test_world();
        assert_eq!(world.user_root_slot_count(), 4);
        const { assert!(USER_ROOT_SLOTS_MAX >= 5) };
        assert_eq!(
            PhysicsWorld::user_root_entity_id(3),
            USER_ROOT_ENTITY_FIRST + 3
        );
        let n = world.user_root_slot_count();
        let mut xs = Vec::with_capacity(n);
        for slot in 0..n {
            let (x, y, z) = world.user_root_translation(slot);
            assert!((y - 1.6).abs() < 0.01 && (z - 6.0).abs() < 0.01);
            xs.push(x);
        }
        xs.sort_by(|a, b| a.partial_cmp(b).unwrap());
        for w in xs.windows(2) {
            assert!(
                (w[1] - w[0]).abs() > 0.5,
                "spawns should differ on X: {xs:?}"
            );
        }
    }

    #[test]
    fn slots_follow_manifest_entry_count() {
        let mut entries = demo_entries_for_test();
        entries.retain(|e| {
            !alchemy_protocol::scene_entry_is_user_root(e.entity_id)
                || e.entity_id <= USER_ROOT_ENTITY_FIRST + 1
        });
        let world = PhysicsWorld::from_entries(&entries, LocoMode::Noclip, DEFAULT_MOVE_SPEED);
        assert_eq!(world.user_root_slot_count(), 2);
        assert_eq!(
            PhysicsWorld::user_root_entity_id(1),
            USER_ROOT_ENTITY_FIRST + 1
        );
    }

    #[test]
    fn five_manifest_roots_within_band_ceiling() {
        let mut entries = demo_entries_for_test();
        entries.push(SceneEntry {
            entity_id: USER_ROOT_ENTITY_FIRST + 4,
            x: 4.5,
            y: 1.6,
            z: 6.0,
            r: 0.8,
            g: 0.8,
            b: 0.2,
            a: 1.0,
            ..Default::default()
        });
        let world = PhysicsWorld::from_entries(&entries, LocoMode::Noclip, DEFAULT_MOVE_SPEED);
        assert_eq!(world.user_root_slot_count(), 5);
        assert_eq!(
            PhysicsWorld::user_root_entity_id(4),
            USER_ROOT_ENTITY_FIRST + 4
        );
    }

    #[test]
    fn unspecified_maps_to_noclip() {
        assert_eq!(LocoMode::from_proto(0), LocoMode::Noclip);
        assert_eq!(LocoMode::from_proto(1), LocoMode::Noclip);
    }

    #[test]
    fn boxes_spawn_as_3x3_grabbable() {
        let world = test_world();
        assert_eq!(world.boxes.len(), 9);
        assert_eq!(world.boxes.entity_ids[0], 10);
        assert_eq!(world.boxes.entity_ids[8], 18);
        assert!(world.boxes.grabbable.iter().all(|&g| g));
        let (x4, y4, z4) = world.box_pose(4).0;
        assert!((y4 - 0.25).abs() < 0.01);
        assert!(
            x4.abs() < 0.01 && (z4 - 3.5).abs() < 0.01,
            "center=({x4},{z4})"
        );
    }

    #[test]
    fn center_box_spawns_with_manifest_trs() {
        let world = test_world();
        let id = world.boxes.entity_ids[4];
        let scale = world.scale_for(id);
        assert!((scale[0] - 1.5).abs() < 1e-5);
        assert!((scale[1] - 1.0).abs() < 1e-5);
        assert!((scale[2] - 1.0).abs() < 1e-5);
        let (_, q) = world.box_pose(4);
        assert!((q[0]).abs() < 1e-4);
        assert!((q[1] - 0.38268343).abs() < 1e-4);
        assert!((q[2]).abs() < 1e-4);
        assert!((q[3] - 0.9238795).abs() < 1e-4);
        assert!((world.boxes.half_extents[4] - 0.25).abs() < 1e-6);
    }

    #[test]
    fn non_grabbable_box_is_fixed() {
        let entries = vec![
            SceneEntry {
                entity_id: USER_ROOT_ENTITY_FIRST,
                ..Default::default()
            },
            SceneEntry {
                entity_id: 20,
                blake3: String::new(),
                x: 0.0,
                y: 0.25,
                z: 0.0,
                half_extent: 0.25,
                flags: 0,
                r: 1.0,
                g: 1.0,
                b: 1.0,
                a: 1.0,
                ..Default::default()
            },
        ];
        let mut world = PhysicsWorld::from_entries(&entries, LocoMode::Noclip, DEFAULT_MOVE_SPEED);
        assert_eq!(world.boxes.len(), 1);
        assert!(!world.boxes.grabbable[0]);
        let (_, y0, _) = world.box_pose(0).0;
        for _ in 0..120 {
            world.step();
        }
        let (_, y1, _) = world.box_pose(0).0;
        assert!(
            (y1 - y0).abs() < 1e-4,
            "Fixed non-Grab box must not settle under gravity: {y0} → {y1}"
        );
    }

    #[test]
    fn boxes_rest_on_floor_after_settle() {
        let mut world = test_world();
        for _ in 0..300 {
            world.step();
        }
        for i in 0..world.boxes.len() {
            let (_, y, _) = world.box_pose(i).0;
            assert!(y > 0.15 && y < 0.6, "box {i} expected on floor, y={y}");
        }
    }

    #[test]
    fn grab_hold_follows_aim_then_drops() {
        let mut world = test_world();
        for _ in 0..120 {
            world.step();
        }
        let id = 14u32;
        let (bx, by, bz) = world.box_pose(4).0;
        let aim0 = (bx - 0.1, by, bz - 0.2);
        world.apply_grab(0, true, id, aim0);
        assert_eq!(world.grab_entity_id(0), Some(id));
        let holder = PhysicsWorld::user_root_entity_id(0);
        assert_eq!(world.slot_graph().parent(id), Some(holder));
        let aim1 = (aim0.0 + 3.0, aim0.1 + 2.5, aim0.2);
        for _ in 0..10 {
            world.apply_grab(0, true, id, aim1);
            world.step();
        }
        let (x, y, z) = world.box_pose(4).0;
        assert!(
            (x - (bx + 3.0)).abs() < 0.05 && (y - (by + 2.5)).abs() < 0.05 && (z - bz).abs() < 0.05,
            "expected follow, got ({x},{y},{z})"
        );
        world.apply_grab(0, false, 0, aim1);
        assert!(world.grab_entity_id(0).is_none());
        assert_eq!(world.slot_graph().parent(id), None);
        for _ in 0..300 {
            world.step();
        }
        let (_, y_after, _) = world.box_pose(4).0;
        assert!(y_after < 1.0 && y_after < y - 1.0);
    }

    #[test]
    fn grab_syncs_slot_graph_held_parent() {
        let mut world = test_world();
        for _ in 0..60 {
            world.step();
        }
        let id = 14u32;
        let (bx, by, bz) = world.box_pose(4).0;
        let holder = PhysicsWorld::user_root_entity_id(0);
        world.apply_grab(0, true, id, (bx, by, bz));
        assert!(world.slot_graph().is_held_by(id, holder));
        world.release_slot(0);
        assert_eq!(world.slot_graph().parent(id), None);
    }

    #[test]
    fn pass_held_moves_to_other_slot_place_clears() {
        let mut world = test_world();
        for _ in 0..60 {
            world.step();
        }
        let id = 14u32;
        let (bx, by, bz) = world.box_pose(4).0;
        let (x0, y0, z0) = world.user_root_translation(0);
        world.teleport_user_root(1, (x0 + 0.5, y0, z0));
        world.apply_grab(0, true, id, (bx, by, bz));
        assert_eq!(world.grab_entity_id(0), Some(id));
        assert!(world.pass_held(0, 1));
        assert!(world.grab_entity_id(0).is_none());
        assert_eq!(world.grab_entity_id(1), Some(id));
        let holder1 = PhysicsWorld::user_root_entity_id(1);
        assert!(world.slot_graph().is_held_by(id, holder1));
        assert!(world.place_held(1));
        assert!(world.grab_entity_id(1).is_none());
        assert_eq!(world.slot_graph().parent(id), None);
    }

    #[test]
    fn grab_ignores_non_box_entity() {
        let mut world = test_world();
        world.apply_grab(0, true, 1, (0.0, 1.0, 0.0));
        assert!(world.grab_entity_id(0).is_none());
    }

    #[test]
    fn release_slot_clears_grab_and_resets_spawn() {
        let mut world =
            PhysicsWorld::from_entries(&demo_entries_for_test(), LocoMode::Noclip, 10.0);
        for _ in 0..30 {
            world.step();
        }
        let id = 14u32;
        let (bx, by, bz) = world.box_pose(4).0;
        world.apply_grab(0, true, id, (bx, by, bz));
        assert_eq!(world.grab_entity_id(0), Some(id));
        let dt = 1.0 / 60.0;
        for _ in 0..60 {
            world.apply_user_intent(0, 1.0, 0.0, 0.0, dt);
            world.apply_grab(0, true, id, (bx + 1.0, by, bz));
            world.step();
        }
        world.release_slot(0);
        assert!(world.grab_entity_id(0).is_none());
        let (x, y, z) = world.user_root_translation(0);
        let (sx, sy, sz) = world.user_root_spawns[0];
        assert!((x - sx).abs() < 0.01 && (y - sy).abs() < 0.01 && (z - sz).abs() < 0.01);
    }

    #[test]
    fn grab_no_steal_from_other_slot() {
        let mut world = test_world();
        for _ in 0..60 {
            world.step();
        }
        let id = 14u32;
        let (bx, by, bz) = world.box_pose(4).0;
        world.apply_grab(0, true, id, (bx, by, bz));
        world.apply_grab(1, true, id, (bx + 1.0, by, bz));
        assert!(world.grab_entity_id(1).is_none());
        assert_eq!(world.grab_entity_id(0), Some(id));
    }

    #[test]
    fn grab_start_rejects_when_player_far() {
        let mut world =
            PhysicsWorld::from_entries(&demo_entries_for_test(), LocoMode::Noclip, 20.0);
        let dt = 1.0 / 60.0;
        for _ in 0..120 {
            world.apply_user_intent(0, 1.0, 0.0, 0.0, dt);
            world.step();
        }
        let id = 14u32;
        let (bx, by, bz) = world.box_pose(4).0;
        let (ux, uy, uz) = world.user_root_translation(0);
        assert!(!crate::finite::within_grab_start_distance(
            (ux, uy, uz),
            (bx, by, bz)
        ));
        world.apply_grab(0, true, id, (bx, by, bz));
        assert!(world.grab_entity_id(0).is_none());
    }

    #[test]
    fn spawn_mesh_entry_adds_second_mesh() {
        let mut world = PhysicsWorld::from_entries(
            &demo_entries_for_test(),
            LocoMode::Noclip,
            DEFAULT_MOVE_SPEED,
        );
        let before = world.mesh_entity_ids().count();
        let entry = SceneEntry {
            entity_id: 100,
            blake3: "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb".into(),
            x: 1.0,
            y: 2.0,
            z: 3.0,
            half_extent: 0.0,
            flags: 0,
            r: 1.0,
            g: 1.0,
            b: 1.0,
            a: 1.0,
            qx: 0.0,
            qy: 0.0,
            qz: 0.0,
            qw: 1.0,
            sx: 1.0,
            sy: 1.0,
            sz: 1.0,
        };
        assert!(world.spawn_mesh_entry(&entry));
        assert_eq!(world.mesh_entity_ids().count(), before + 1);
        assert!(world.mesh_pose_for(100).is_some());
        assert!(!world.spawn_mesh_entry(&entry));
        assert_eq!(world.mesh_entity_ids().count(), before + 1);
        assert!(world.despawn_mesh_entity(100));
        assert_eq!(world.mesh_entity_ids().count(), before);
        assert!(world.mesh_pose_for(100).is_none());
        assert!(!world.despawn_mesh_entity(100));
    }

    #[test]
    fn grab_hold_far_aim_does_not_teleport_beyond_budget() {
        let mut world = test_world();
        for _ in 0..120 {
            world.step();
        }
        let id = 14u32;
        let (bx, by, bz) = world.box_pose(4).0;
        let (ux0, uy0, uz0) = world.user_root_translation(0);
        world.apply_grab(0, true, id, (bx, by, bz));
        assert_eq!(world.grab_entity_id(0), Some(id));
        let far = (ux0 + 1e6, uy0, uz0);
        for _ in 0..10 {
            world.apply_grab(0, true, id, far);
            world.step();
        }
        let (x, y, z) = world.box_pose(4).0;
        let (ux, uy, uz) = world.user_root_translation(0);
        let dx = x - ux;
        let dy = y - uy;
        let dz = z - uz;
        let dist = (dx * dx + dy * dy + dz * dz).sqrt();
        let max = crate::finite::MAX_GRAB_START_DISTANCE;
        assert!(
            dist <= 2.0 * max + 0.05,
            "box teleported too far: dist={dist} pos=({x},{y},{z}) root=({ux},{uy},{uz})"
        );
        assert!(x.is_finite() && y.is_finite() && z.is_finite());
        assert!(
            dx > 0.0,
            "far +X aim should keep box on +X side of root, dx={dx}"
        );
    }

    #[test]
    fn grab_hold_opposite_aim_offset_clamp_keeps_two_max_budget() {
        let mut world = test_world();
        for _ in 0..120 {
            world.step();
        }
        let id = 14u32;
        let (bx, by, bz) = world.box_pose(4).0;
        let (ux, uy, uz) = world.user_root_translation(0);
        let vx = bx - ux;
        let vy = by - uy;
        let vz = bz - uz;
        let vlen = (vx * vx + vy * vy + vz * vz).sqrt();
        assert!(
            vlen > 0.05 && vlen < crate::finite::MAX_GRAB_START_DISTANCE,
            "need box inside start gate but not on root, vlen={vlen}"
        );
        let max = crate::finite::MAX_GRAB_START_DISTANCE;
        let inv = max / vlen;
        let aim0 = (ux - vx * inv, uy - vy * inv, uz - vz * inv);
        let raw_ox = bx - aim0.0;
        let raw_oy = by - aim0.1;
        let raw_oz = bz - aim0.2;
        let raw_olen = (raw_ox * raw_ox + raw_oy * raw_oy + raw_oz * raw_oz).sqrt();
        assert!(
            raw_olen > max + 0.05,
            "fixture must exercise offset clamp: raw_olen={raw_olen} max={max}"
        );
        world.apply_grab(0, true, id, aim0);
        assert_eq!(world.grab_entity_id(0), Some(id));
        let far = (ux + vx * 1e6, uy + vy * 1e6, uz + vz * 1e6);
        for _ in 0..10 {
            world.apply_grab(0, true, id, far);
            world.step();
        }
        let (x, y, z) = world.box_pose(4).0;
        let (ux1, uy1, uz1) = world.user_root_translation(0);
        let dx = x - ux1;
        let dy = y - uy1;
        let dz = z - uz1;
        let dist = (dx * dx + dy * dy + dz * dz).sqrt();
        assert!(x.is_finite() && y.is_finite() && z.is_finite());
        assert!(
            dist <= 2.0 * max + 0.05,
            "offset clamp regression? dist={dist} (>2·MAX) pos=({x},{y},{z}) root=({ux1},{uy1},{uz1})"
        );
        assert!(
            dist > max,
            "expected stacked aim+offset near outer shell, dist={dist}"
        );
        let along = dx * vx + dy * vy + dz * vz;
        assert!(along > 0.0, "box should remain on the box side of root");
    }

    #[test]
    fn huge_move_magnitude_does_not_teleport() {
        let mut world = test_world();
        let (x0, y0, z0) = world.user_root_translation(0);
        let dt = 1.0 / 60.0;
        world.apply_user_intent(0, 1e30, 0.0, 0.0, dt);
        world.step();
        let (x1, y1, z1) = world.user_root_translation(0);
        let dx = (x1 - x0).abs();
        assert!(
            dx < DEFAULT_MOVE_SPEED * dt * 1.1,
            "expected clamped step, dx={dx} from ({x0},{y0},{z0}) to ({x1},{y1},{z1})"
        );
    }

    #[test]
    fn xpbd_rows_are_dynamic_for_mesh() {
        let world = test_world();
        let mesh_id = world.mesh_entity_id().unwrap();
        let row = world.entity_row(mesh_id).unwrap().row_id as usize;
        let sheet = world.snapshots().load_curr();
        assert!(sheet.readiness[row].contains(ReadinessMask::DYNAMIC));
        assert!(sheet.world_transforms[row].inv_mass > 0.0);
    }

    #[test]
    fn tick_keeps_prev_curr_distinct_while_falling() {
        let mut world = test_world();
        let mesh_id = world.mesh_entity_id().unwrap();
        let row = world.entity_row(mesh_id).unwrap().row_id as usize;
        for _ in 0..30 {
            world.step();
        }
        let prev = world.snapshots().load_prev().world_transforms[row].position;
        let curr = world.snapshots().load_curr().world_transforms[row].position;
        assert_ne!(prev, curr, "falling mesh should publish distinct Prev/Curr");
    }

    #[test]
    fn mesh_stays_finite_under_intent_pressure() {
        let mut world =
            PhysicsWorld::from_entries(&demo_entries_for_test(), LocoMode::Noclip, 10.0);
        let mesh_id = world.mesh_entity_id().unwrap();
        let row = world.entity_row(mesh_id).unwrap().row_id as usize;
        let dt = 1.0 / 60.0;
        for i in 0..3600 {
            world.apply_user_intent(0, 1.0, 0.5, -0.25, dt);
            world.step();
            let p = world.world.curr.world_transforms[row].position;
            let d = world.world.curr.world_transforms[row].frame_delta_xyz;
            assert!(
                p.iter().all(|v| v.is_finite()) && d.iter().all(|v| v.is_finite()),
                "mesh non-finite at step {i}: pos={p:?} delta={d:?}"
            );
        }
        let (ux, uy, uz) = world.user_root_translation(0);
        assert!(ux.is_finite() && uy.is_finite() && uz.is_finite());
        assert!(
            ux.abs() > 1.0,
            "Intent must actually translate user_root, got ({ux},{uy},{uz})"
        );
    }

    #[test]
    fn mesh_stays_finite_idle_long_run() {
        for substeps in [1u32, 4, 8] {
            let mut world = test_world();
            world.world.substeps = substeps;
            let mesh_id = world.mesh_entity_id().unwrap();
            let row = world.entity_row(mesh_id).unwrap().row_id as usize;
            let mut prev_dx = world.world.curr.world_transforms[row].frame_delta_xyz[0];
            for i in 0..600 {
                world.step();
                let p = world.world.curr.world_transforms[row].position;
                let d = world.world.curr.world_transforms[row].frame_delta_xyz;
                assert!(
                    p.iter().all(|v| v.is_finite()) && d.iter().all(|v| v.is_finite()),
                    "mesh non-finite at idle step {i} substeps={substeps}: pos={p:?} delta={d:?}"
                );
                // Any |dx| growth toward ×substeps is the handoff bug; allow mild noise only.
                if prev_dx.abs() > 1e-6 && d[0].abs() > 1e-6 {
                    let ratio = (d[0] / prev_dx).abs();
                    assert!(
                        ratio < 1.5,
                        "frame_delta amplified at step {i} substeps={substeps}: ratio={ratio} prev={prev_dx} next={}",
                        d[0]
                    );
                }
                assert!(
                    p[0].abs() < 50.0 && p[2].abs() < 50.0,
                    "horizontal explode step {i} substeps={substeps}: pos={p:?}"
                );
                prev_dx = d[0];
            }
        }
    }

    #[test]
    fn sanitize_non_finite_zeros_frame_delta_not_origin_minus_start() {
        let mut world = test_world();
        let mesh_id = world.mesh_entity_id().unwrap();
        let row = world.entity_row(mesh_id).unwrap().row_id as usize;
        // Pre-explode start pose far from origin; Inf end must not become delta≈-start.
        {
            let xf = &mut world.world.curr.world_transforms[row];
            xf.position = [12.0, 3.0, -8.0];
            xf.frame_delta_xyz = [0.5, 0.0, -0.25];
            let mask = world.world.curr.readiness[row];
            world.world.curr.readiness[row] = mask.without_bits(ReadinessMask::SLEEPING);
        }
        world.world.prev.world_transforms[row] = world.world.curr.world_transforms[row];
        world.world.curr.world_transforms[row].position = [f64::INFINITY, 0.25, f64::INFINITY];
        world.step();
        let xf = &world.world.curr.world_transforms[row];
        assert!(
            xf.is_finite(),
            "sanitize must restore finite pose, got pos={:?} delta={:?}",
            xf.position,
            xf.frame_delta_xyz
        );
        assert_eq!(
            xf.frame_delta_xyz,
            [0.0, 0.0, 0.0],
            "sanitized row must keep delta 0 (not origin - start)"
        );
        assert!(
            world.sanitize_scratch.contains(&(row as u32)),
            "sanitize_scratch must record the row for write_frame_deltas"
        );
        let support = PHYS_BALL_RADIUS as f64;
        assert!(
            (xf.position[0] - 0.0).abs() < 1e-9
                && (xf.position[1] - support).abs() < 1e-9
                && (xf.position[2] - 0.0).abs() < 1e-9,
            "expected ground support at origin, got {:?}",
            xf.position
        );
        // Next tick must not re-launch from a huge handoff delta.
        world.step();
        let xf2 = &world.world.curr.world_transforms[row];
        assert!(
            xf2.is_finite(),
            "post-sanitize tick must stay finite: {:?}",
            xf2.position
        );
        assert!(
            xf2.position[0].abs() < 1.0 && xf2.position[2].abs() < 1.0,
            "post-sanitize must not fly away, got {:?}",
            xf2.position
        );
    }

    #[test]
    fn host_body_body_contacts_separate_overlapping_boxes() {
        // P1-2 host: LBVH inequality contacts on (x00ad); boxes must not stay overlapping.
        let mut world = test_world();
        world.world.gravity = [0.0, 0.0, 0.0];
        assert_eq!(world.backend(), SimBackendKind::EpochXpbd);

        let id0 = world.boxes.entity_ids[0];
        let id1 = world.boxes.entity_ids[1];
        let r0 = world.entity_row(id0).unwrap().row_id as usize;
        let r1 = world.entity_row(id1).unwrap().row_id as usize;
        let half = world.boxes.half_extents[0];
        for &row in &[r0, r1] {
            let mask = world.world.curr.readiness[row];
            world.world.curr.readiness[row] = mask
                .with_bits(ReadinessMask::DYNAMIC)
                .without_bits(ReadinessMask::SLEEPING | ReadinessMask::GRABBED);
            world.world.curr.world_transforms[row].inv_mass = 1.0;
            world.world.curr.world_transforms[row].frame_delta_xyz = [0.0, 0.0, 0.0];
        }
        world.world.curr.world_transforms[r0].position = [-0.1, 1.0, 0.0];
        world.world.curr.world_transforms[r1].position = [0.1, 1.0, 0.0];

        for _ in 0..40 {
            world.step();
        }

        let a = world.world.curr.world_transforms[r0].position;
        let b = world.world.curr.world_transforms[r1].position;
        let dx = a[0] - b[0];
        let dy = a[1] - b[1];
        let dz = a[2] - b[2];
        let dist = (dx * dx + dy * dy + dz * dz).sqrt();
        let radius = (3.0_f64 * (half as f64).powi(2)).sqrt();
        let min_sep = 2.0 * radius;
        assert!(
            dist + 1e-3 >= min_sep,
            "host boxes still overlapping: dist={dist} min_sep={min_sep} a={a:?} b={b:?}"
        );
        assert!(a.iter().chain(b.iter()).all(|v| v.is_finite()));
    }

    fn test_world_mvcc() -> PhysicsWorld {
        PhysicsWorld::from_entries_with_backend(
            &demo_entries_for_test(),
            LocoMode::Noclip,
            DEFAULT_MOVE_SPEED,
            SimBackendKind::MvccXpbd,
        )
        .expect("demo scene fits one MVCC page")
    }

    #[test]
    fn mvcc_backend_steps_and_rejects_nan_ingest() {
        let mut world = test_world_mvcc();
        assert_eq!(world.backend(), SimBackendKind::MvccXpbd);
        assert!(world.mvcc_page().is_some());

        for _ in 0..30 {
            world.step();
        }
        let (x, y, z) = world.moving_translation();
        assert!(
            x.is_finite() && y.is_finite() && z.is_finite(),
            "mvcc_xpbd mesh must stay finite ({x},{y},{z})"
        );

        let page = world.mvcc_page().unwrap();
        assert!(
            page.active_mask.iter().any(|&w| w != 0),
            "mvcc page should have active entities after step"
        );

        let nan = KinematicPoseF32::xyz_quat(f32::NAN, 0.0, 0.0, 0.0, 0.0, 0.0, 1.0);
        assert!(
            !world.try_ingest_mvcc_kinematic_for_test(0, nan),
            "NaN kinematic ingest must be rejected"
        );
    }

    #[test]
    fn mvcc_backend_body_body_contacts_separate_boxes() {
        let mut world = test_world_mvcc();
        world.world.gravity = [0.0, 0.0, 0.0];
        if let Some(mvcc) = world.mvcc.as_mut() {
            mvcc.adapter.set_gravity([0.0, 0.0, 0.0]);
        }

        let id0 = world.boxes.entity_ids[0];
        let id1 = world.boxes.entity_ids[1];
        let r0 = world.entity_row(id0).unwrap().row_id as usize;
        let r1 = world.entity_row(id1).unwrap().row_id as usize;
        let half = world.boxes.half_extents[0];
        for &row in &[r0, r1] {
            let mask = world.world.curr.readiness[row];
            world.world.curr.readiness[row] = mask
                .with_bits(ReadinessMask::DYNAMIC)
                .without_bits(ReadinessMask::SLEEPING | ReadinessMask::GRABBED);
            world.world.curr.world_transforms[row].inv_mass = 1.0;
            world.world.curr.world_transforms[row].frame_delta_xyz = [0.0, 0.0, 0.0];
        }
        world.world.curr.world_transforms[r0].position = [-0.1, 1.0, 0.0];
        world.world.curr.world_transforms[r1].position = [0.1, 1.0, 0.0];

        for _ in 0..40 {
            world.step();
        }

        let a = world.world.curr.world_transforms[r0].position;
        let b = world.world.curr.world_transforms[r1].position;
        let dx = a[0] - b[0];
        let dy = a[1] - b[1];
        let dz = a[2] - b[2];
        let dist = (dx * dx + dy * dy + dz * dz).sqrt();
        let radius = (3.0_f64 * (half as f64).powi(2)).sqrt();
        let min_sep = 2.0 * radius;
        assert!(
            dist + 1e-3 >= min_sep,
            "mvcc host boxes still overlapping: dist={dist} min_sep={min_sep}"
        );
    }

    #[test]
    fn mvcc_ground_contact_tick_zeros_vertical_frame_delta() {
        let mut world = PhysicsWorld::from_entries_with_backend(
            &demo_entries_for_test(),
            LocoMode::Walk,
            DEFAULT_MOVE_SPEED,
            SimBackendKind::MvccXpbd,
        )
        .unwrap();
        let mesh_id = world.mesh_entity_id().unwrap();
        let row = world.entity_row(mesh_id).unwrap().row_id as usize;
        let min_y = GROUND_Y + PHYS_BALL_RADIUS as f64;
        {
            let xf = &mut world.world.curr.world_transforms[row];
            xf.position = [0.0, min_y + 0.02, 0.0];
            xf.frame_delta_xyz = [0.0, -1.0, 0.0];
            let mask = world.world.curr.readiness[row];
            world.world.curr.readiness[row] = mask.without_bits(ReadinessMask::SLEEPING);
        }
        world.step();
        let y_after = world.world.curr.world_transforms[row].position[1];
        let dy = world.world.curr.world_transforms[row].frame_delta_xyz[1];
        assert!(
            (y_after - min_y).abs() < 1e-9,
            "mvcc contact tick must clamp to ground, y_after={y_after}"
        );
        assert!(
            world.ground_scratch.contains(&(row as u32)),
            "mvcc contact tick must mark row grounded"
        );
        assert!(
            dy.abs() <= 1e-6,
            "mvcc vertical frame_delta must be 0, got {dy}"
        );
    }

    #[test]
    fn mvcc_grab_hold_follows_aim_then_drops() {
        let mut world = test_world_mvcc();
        for _ in 0..120 {
            world.step();
        }
        let id = 14u32;
        let (bx, by, bz) = world.box_pose(4).0;
        let aim0 = (bx - 0.1, by, bz - 0.2);
        world.apply_grab(0, true, id, aim0);
        assert_eq!(world.grab_entity_id(0), Some(id));
        let row = world.entity_row(id).unwrap().row_id as usize;
        world.step();
        assert_eq!(
            world.mvcc_page().unwrap().holder_at(row),
            0,
            "page holder must mirror Grab slot 0"
        );
        let aim1 = (aim0.0 + 3.0, aim0.1 + 2.5, aim0.2);
        for _ in 0..10 {
            world.apply_grab(0, true, id, aim1);
            world.step();
        }
        let (x, y, z) = world.box_pose(4).0;
        assert!(
            (x - (bx + 3.0)).abs() < 0.05 && (y - (by + 2.5)).abs() < 0.05 && (z - bz).abs() < 0.05,
            "mvcc expected grab follow, got ({x},{y},{z})"
        );
        world.apply_grab(0, false, 0, aim1);
        assert!(world.grab_entity_id(0).is_none());
        world.step();
        assert_eq!(
            world.mvcc_page().unwrap().holder_at(row),
            NO_HOLDER,
            "release must clear page holder"
        );
    }

    #[test]
    fn mvcc_page_holder_tracks_pass_held_slot() {
        let mut world = test_world_mvcc();
        for _ in 0..60 {
            world.step();
        }
        let id = 14u32;
        let (bx, by, bz) = world.box_pose(4).0;
        let (x0, y0, z0) = world.user_root_translation(0);
        world.teleport_user_root(1, (x0 + 0.5, y0, z0));
        world.apply_grab(0, true, id, (bx, by, bz));
        let row = world.entity_row(id).unwrap().row_id as usize;
        world.step();
        assert_eq!(world.mvcc_page().unwrap().holder_at(row), 0);
        assert!(world.pass_held(0, 1));
        world.step();
        assert_eq!(
            world.mvcc_page().unwrap().holder_at(row),
            1,
            "page holder must follow pass_held to slot 1"
        );
    }

    fn assert_worlds_bit_equal(a: &PhysicsWorld, b: &PhysicsWorld) {
        assert_eq!(a.entity_count(), b.entity_count());
        let mut ids: Vec<u32> = a.entity_rows.keys().copied().collect();
        ids.sort_unstable();
        for id in ids {
            let ra = a.entity_row(id).unwrap().row_id as usize;
            let rb = b.entity_row(id).unwrap().row_id as usize;
            let xa = &a.world.curr.world_transforms[ra];
            let xb = &b.world.curr.world_transforms[rb];
            for i in 0..3 {
                assert_eq!(
                    xa.position[i].to_bits(),
                    xb.position[i].to_bits(),
                    "pos mismatch entity={id} axis={i}"
                );
            }
            for i in 0..4 {
                assert_eq!(
                    xa.rotation[i].to_bits(),
                    xb.rotation[i].to_bits(),
                    "rot mismatch entity={id} axis={i}"
                );
            }
            for i in 0..3 {
                assert_eq!(
                    xa.frame_delta_xyz[i].to_bits(),
                    xb.frame_delta_xyz[i].to_bits(),
                    "delta mismatch entity={id} axis={i}"
                );
            }
            let ma = a.world.curr.readiness[ra];
            let mb = b.world.curr.readiness[rb];
            assert_eq!(
                ma.contains(ReadinessMask::SLEEPING),
                mb.contains(ReadinessMask::SLEEPING),
                "SLEEPING mismatch entity={id}"
            );
            assert_eq!(
                ma.contains(ReadinessMask::GRABBED),
                mb.contains(ReadinessMask::GRABBED),
                "GRABBED mismatch entity={id}"
            );
        }
    }

    fn planned_handover_continues_for(backend: SimBackendKind) {
        let entries = demo_entries_for_test();
        let mut old = PhysicsWorld::from_entries_with_backend(
            &entries,
            LocoMode::Noclip,
            DEFAULT_MOVE_SPEED,
            backend,
        )
        .expect("old world");
        for _ in 0..90 {
            old.step();
        }
        // Grab a box so holder survives export/import.
        let id = old.boxes.entity_ids[0];
        let (bx, by, bz) = old.box_pose(0).0;
        old.apply_grab(0, true, id, (bx, by, bz));
        // Force SLEEPING on a non-held mesh so flags→readiness restore is covered.
        if let Some(mesh_id) = old.mesh_entity_id() {
            let row = old.entity_row(mesh_id).unwrap().row_id as usize;
            let mask = old.world.curr.readiness[row];
            old.world.curr.readiness[row] = mask.with_bits(ReadinessMask::SLEEPING);
        }
        old.step();
        let sim_tick = 91u64;
        let exported = old.export_sim_state("local", 1, 1, sim_tick, 45);
        assert!(!exported.entities.is_empty());
        assert!(exported.entities.iter().any(|e| e.holder == 1));

        let key = alchemy_protocol::wire_crypto::generate_signing_key();
        let mut signed = exported.clone();
        alchemy_protocol::wire_crypto::sign_sim_state(&key, &mut signed);
        let plain = signed.encode_to_vec();
        let room_key = [0x42u8; 32];
        let env = alchemy_protocol::wire_aead::seal(
            &room_key,
            "local",
            0,
            alchemy_protocol::wire_aead::AeadKind::SimState,
            &plain,
        )
        .expect("seal");
        let opened = alchemy_protocol::wire_aead::open(
            &room_key,
            "local",
            0,
            alchemy_protocol::wire_aead::AeadKind::SimState,
            &env,
        )
        .expect("open");
        let decoded = alchemy_protocol::decode_limits::decode_sim_state_checked(&opened)
            .expect("decode+limits");
        alchemy_protocol::wire_crypto::verify_sim_state(
            &alchemy_protocol::wire_crypto::public_key_bytes(&key),
            &decoded,
        )
        .expect("verify");
        assert!(alchemy_protocol::wire_crypto::sim_state_entities_finite(
            &decoded
        ));

        let mut resumed = PhysicsWorld::from_sim_state_with_backend(
            &entries,
            LocoMode::Noclip,
            DEFAULT_MOVE_SPEED,
            backend,
            &decoded,
            SimStateImportKind::Planned,
        )
        .expect("resume");
        assert_eq!(resumed.grab_entity_id(0), Some(id));

        let mut control = PhysicsWorld::from_sim_state_with_backend(
            &entries,
            LocoMode::Noclip,
            DEFAULT_MOVE_SPEED,
            backend,
            &decoded,
            SimStateImportKind::Planned,
        )
        .expect("control twin");

        for _ in 0..60 {
            resumed.step();
            control.step();
            old.step();
        }
        assert_worlds_bit_equal(&resumed, &control);
        // Same binary: resumed twin matches the continuously stepped old host.
        assert_worlds_bit_equal(&resumed, &old);
    }

    #[test]
    fn planned_handover_continues_trajectory_epoch() {
        planned_handover_continues_for(SimBackendKind::EpochXpbd);
    }

    #[test]
    fn planned_handover_continues_trajectory_mvcc() {
        planned_handover_continues_for(SimBackendKind::MvccXpbd);
    }

    #[test]
    fn sim_state_flags_zero_keeps_cold_readiness() {
        let entries = demo_entries_for_test();
        let mut old = PhysicsWorld::from_entries_with_backend(
            &entries,
            LocoMode::Noclip,
            DEFAULT_MOVE_SPEED,
            SimBackendKind::EpochXpbd,
        )
        .expect("old");
        for _ in 0..30 {
            old.step();
        }
        let mesh_id = old.mesh_entity_id().expect("mesh");
        let row_before = old.entity_row(mesh_id).unwrap().row_id as usize;
        assert!(old.world.curr.readiness[row_before].contains(ReadinessMask::ALLOCATED));
        assert!(old.world.curr.readiness[row_before].is_ready(ReadinessMask::DYNAMIC));

        let mut crash_seed = old.export_sim_state("local", 1, 1, 30, 15);
        for e in &mut crash_seed.entities {
            e.flags = 0; // crash snapshot contract
            e.dx = 0.0;
            e.dy = 0.0;
            e.dz = 0.0;
        }
        let resumed = PhysicsWorld::from_sim_state_with_backend(
            &entries,
            LocoMode::Noclip,
            DEFAULT_MOVE_SPEED,
            SimBackendKind::EpochXpbd,
            &crash_seed,
            SimStateImportKind::CrashPoseOnly,
        )
        .expect("resume");
        let row = resumed.entity_row(mesh_id).unwrap().row_id as usize;
        assert!(
            resumed.world.curr.readiness[row].contains(ReadinessMask::ALLOCATED),
            "flags==0 must not wipe ALLOCATED"
        );
        assert!(
            resumed.world.curr.readiness[row].is_ready(ReadinessMask::DYNAMIC),
            "flags==0 must not wipe DYNAMIC"
        );
        assert!(
            PhysicsWorld::from_sim_state_with_backend(
                &entries,
                LocoMode::Noclip,
                DEFAULT_MOVE_SPEED,
                SimBackendKind::EpochXpbd,
                &crash_seed,
                SimStateImportKind::Planned,
            )
            .is_err(),
            "planned must reject all-zero flags"
        );

        // CrashPoseOnly must ignore non-zero wire flags (keep cold mask).
        let mut sneaky = crash_seed.clone();
        for e in &mut sneaky.entities {
            e.flags = ReadinessMask::SLEEPING as u32;
        }
        let resumed2 = PhysicsWorld::from_sim_state_with_backend(
            &entries,
            LocoMode::Noclip,
            DEFAULT_MOVE_SPEED,
            SimBackendKind::EpochXpbd,
            &sneaky,
            SimStateImportKind::CrashPoseOnly,
        )
        .expect("crash ignores flags");
        let row2 = resumed2.entity_row(mesh_id).unwrap().row_id as usize;
        assert!(
            !resumed2.world.curr.readiness[row2].contains(ReadinessMask::SLEEPING),
            "CrashPoseOnly must not apply wire SLEEPING"
        );
        assert!(resumed2.world.curr.readiness[row2].is_ready(ReadinessMask::DYNAMIC));
    }

    #[test]
    fn sim_state_restores_sleeping_readiness_flags() {
        let entries = demo_entries_for_test();
        let mut old = PhysicsWorld::from_entries_with_backend(
            &entries,
            LocoMode::Noclip,
            DEFAULT_MOVE_SPEED,
            SimBackendKind::EpochXpbd,
        )
        .expect("old");
        let mesh_id = old.mesh_entity_id().expect("mesh");
        let row = old.entity_row(mesh_id).unwrap().row_id as usize;
        let mask = old.world.curr.readiness[row];
        old.world.curr.readiness[row] = mask.with_bits(ReadinessMask::SLEEPING);
        let exported = old.export_sim_state("local", 1, 1, 1, 0);
        let e = exported
            .entities
            .iter()
            .find(|e| e.id == mesh_id)
            .expect("mesh entity");
        assert_ne!(e.flags & (ReadinessMask::SLEEPING as u32), 0);

        let resumed = PhysicsWorld::from_sim_state_with_backend(
            &entries,
            LocoMode::Noclip,
            DEFAULT_MOVE_SPEED,
            SimBackendKind::EpochXpbd,
            &exported,
            SimStateImportKind::Planned,
        )
        .expect("resume");
        let row2 = resumed.entity_row(mesh_id).unwrap().row_id as usize;
        assert!(
            resumed.world.curr.readiness[row2].contains(ReadinessMask::SLEEPING),
            "SLEEPING must survive sim_state roundtrip"
        );
    }
}
