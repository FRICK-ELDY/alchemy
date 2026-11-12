//! Ring 3 epoch view — GPU interpolate, Hierarchy virtual scroll, Tape X-Ray.
//!
//! Guarantees: GPU path is appearance-only (no readback); Hierarchy walks
//! `euler_order` with O(1) collapse skips; X-Ray formats ReadinessMask / Prev /
//! Curr / depth / Morton for the selected `row_id`; SceneGpu binds interpolate
//! buffers from published faces (x0090) and draws mesh/box TRS from `out`
//! storage (x0091; full active pack).
//! Display α / PrevCurr / CPU pose cache are frozen once per display frame
//! ([`EpochViewState::begin_display_frame`]); Grab/GPU/color/eye share that snap (x0092).
//! Non-guarantees: full Spatial Lens UI; color on tape; topology-driven draw; TPS/Live camera.

use std::collections::{BTreeSet, HashMap, HashSet};
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Instant;

use alchemy_shared_sim::bridge::{EpochFaces, EpochSnapshotPair};
use alchemy_shared_sim::spatial::{morton_code30, SceneBounds};
use alchemy_shared_sim::tape::{EpochSheet, ReadinessMask, WorldTransformf64};
use alchemy_shared_sim::topology::FlatTopology;
use bytemuck::{Pod, Zeroable};

use crate::epoch_mirror::EpochFrameMirror;

/// GPU-side packed transform (f32) for storage-buffer interpolate — no CPU readback.
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, Pod, Zeroable)]
pub struct GpuEpochRow {
    pub pos: [f32; 3],
    pub _pad0: f32,
    pub rot: [f32; 4],
    pub scale: [f32; 3],
    pub row_id: u32,
}

impl GpuEpochRow {
    pub fn from_world(row_id: u32, xf: &WorldTransformf64) -> Self {
        Self {
            pos: [
                xf.position[0] as f32,
                xf.position[1] as f32,
                xf.position[2] as f32,
            ],
            _pad0: 0.0,
            rot: xf.rotation,
            scale: xf.scale,
            row_id,
        }
    }
}

/// WGSL: appearance-only Prev/Curr lerp. Never writes back to CPU.
pub const INTERPOLATE_WGSL: &str = r#"
struct EpochRow {
    pos: vec3<f32>,
    _pad0: f32,
    rot: vec4<f32>,
    scale: vec3<f32>,
    row_id: u32,
};

struct Params {
    alpha: f32,
    row_count: u32,
    _pad: vec2<f32>,
};

@group(0) @binding(0) var<storage, read> prev_rows: array<EpochRow>;
@group(0) @binding(1) var<storage, read> curr_rows: array<EpochRow>;
@group(0) @binding(2) var<storage, read_write> out_rows: array<EpochRow>;
@group(0) @binding(3) var<uniform> params: Params;

@compute @workgroup_size(64)
fn interpolate(@builtin(global_invocation_id) gid: vec3<u32>) {
    let i = gid.x;
    if (i >= params.row_count) { return; }
    let a = params.alpha;
    let p = prev_rows[i];
    let c = curr_rows[i];
    var o: EpochRow;
    o.pos = mix(p.pos, c.pos, a);
    // Hemisphere-aware nlerp: flip prev quat if dot < 0 so we take the short arc.
    var pr = p.rot;
    if (dot(pr, c.rot) < 0.0) {
        pr = -pr;
    }
    o.rot = normalize(mix(pr, c.rot, a));
    // Match CPU twin [`lerp_gpu_rows`]: near-zero scale → identity before consumers.
    var sc = mix(p.scale, c.scale, a);
    if (dot(sc, sc) < 1e-12) {
        sc = vec3<f32>(1.0, 1.0, 1.0);
    }
    o.scale = sc;
    o.row_id = c.row_id;
    o._pad0 = 0.0;
    out_rows[i] = o;
}
"#;

/// One display-frame freeze: same faces + α + packed rows + entity pose cache.
///
/// Grab / GPU upload / coin color must all read this — not a fresh `Instant::now()`.
/// Shared as [`Arc`] so readers (per-mesh coin color, Grab list, sync) cheaply clone the pointer.
#[derive(Debug)]
pub struct DisplayFrame {
    pub faces: Arc<EpochFaces>,
    pub alpha: f32,
    pub prev_rows: Vec<GpuEpochRow>,
    pub curr_rows: Vec<GpuEpochRow>,
    /// `entity_id` → lerped row (full active pack).
    pub poses: HashMap<u32, GpuEpochRow>,
}

/// Build a display-frame snap from one `load_pair` + one `now` (testable pure helper).
pub fn build_display_frame(faces: Arc<EpochFaces>, now: Instant) -> Arc<DisplayFrame> {
    let alpha = crate::epoch_mirror::epoch_interp_alpha(
        crate::epoch_mirror::published_instant(faces.published_at_ns),
        now,
    );
    let ids = aligned_row_ids(&faces.prev, &faces.curr, false);
    let (prev_rows, curr_rows) = pack_aligned_gpu_faces(&faces.prev, &faces.curr, &ids);
    let mut poses = HashMap::with_capacity(curr_rows.len());
    for (p, c) in prev_rows.iter().zip(curr_rows.iter()) {
        let out = lerp_gpu_rows(p, c, alpha);
        if let Some(&eid) = faces.curr.owner_scope.get(out.row_id as usize) {
            poses.insert(eid, out);
        }
    }
    Arc::new(DisplayFrame {
        faces,
        alpha,
        prev_rows,
        curr_rows,
        poses,
    })
}

/// Owns lock-free snapshots + topology for Hierarchy / X-Ray.
pub struct EpochViewState {
    pub snapshots: Arc<EpochSnapshotPair>,
    pub topology: Mutex<FlatTopology>,
    /// Selected row for X-Ray (`u32::MAX` = none).
    pub xray_row: AtomicU32,
    pub scene_bounds: Mutex<SceneBounds>,
    /// Frame → tape kinematic mirror (x0090).
    pub mirror: Mutex<EpochFrameMirror>,
    /// Frozen display faces/α/poses for the current client frame (x0091).
    display_frame: Mutex<Option<Arc<DisplayFrame>>>,
}

impl EpochViewState {
    pub fn new() -> Self {
        Self {
            snapshots: Arc::new(EpochSnapshotPair::new(
                EpochSheet::default(),
                EpochSheet::default(),
            )),
            topology: Mutex::new(FlatTopology::default()),
            xray_row: AtomicU32::new(u32::MAX),
            scene_bounds: Mutex::new(SceneBounds::default()),
            mirror: Mutex::new(EpochFrameMirror::default()),
            display_frame: Mutex::new(None),
        }
    }

    /// Freeze Prev/Curr + α + pose cache for this display frame (call once at frame start).
    pub fn begin_display_frame(&self, now: Instant) {
        let faces = self.snapshots.load_pair();
        let frame = build_display_frame(faces, now);
        if let Ok(mut slot) = self.display_frame.lock() {
            *slot = Some(frame);
        }
    }

    /// Keep existing freeze; if none, begin with `now` (sync / mid-frame Grab fallback).
    ///
    /// Non-Redraw input that calls Grab reuses the **last painted** freeze — aligned to
    /// what the user still sees until the next `begin_display_frame` on Redraw/XR frame.
    pub fn ensure_display_frame(&self, now: Instant) {
        let _ = self.with_display_frame(now, |_| ());
    }

    /// Ensure freeze, then run `f` under a single Mutex hold.
    pub fn with_display_frame<R>(
        &self,
        now: Instant,
        f: impl FnOnce(&DisplayFrame) -> R,
    ) -> Option<R> {
        let mut slot = self.display_frame.lock().ok()?;
        if slot.is_none() {
            let faces = self.snapshots.load_pair();
            *slot = Some(build_display_frame(faces, now));
        }
        Some(f(slot.as_ref()?))
    }

    /// Cheap [`Arc`] clone of the frozen frame (ensure + clone under one lock).
    pub fn display_frame_snapshot(&self) -> Option<Arc<DisplayFrame>> {
        let mut slot = self.display_frame.lock().ok()?;
        if slot.is_none() {
            let faces = self.snapshots.load_pair();
            *slot = Some(build_display_frame(faces, Instant::now()));
        }
        slot.as_ref().map(Arc::clone)
    }

    pub fn set_xray_row(&self, row: Option<u32>) {
        self.xray_row
            .store(row.unwrap_or(u32::MAX), Ordering::Relaxed);
    }

    pub fn xray_row(&self) -> Option<u32> {
        let v = self.xray_row.load(Ordering::Relaxed);
        if v == u32::MAX {
            None
        } else {
            Some(v)
        }
    }
}

impl Default for EpochViewState {
    fn default() -> Self {
        Self::new()
    }
}

fn row_is_live(sheet: &EpochSheet, row: u32) -> bool {
    let idx = row as usize;
    idx < sheet.readiness.len() && !sheet.readiness[idx].contains(ReadinessMask::TOMBSTONE)
}

/// Sentinel pack index: vertex shader uses `model_override` instead of `epoch_out`.
pub const PACK_INDEX_MODEL_OVERRIDE: u32 = u32::MAX;

/// Sorted `row_id` list shared by Prev/Curr packs so GPU index `i` is the same entity.
///
/// `dirty_only`: union of both sheets' dirty bits (still one aligned id list).
/// Tombstoned rows are dropped. **Draw path (x0091) uses full active** (`dirty_only=false`).
pub fn aligned_row_ids(prev: &EpochSheet, curr: &EpochSheet, dirty_only: bool) -> Vec<u32> {
    let mut ids = BTreeSet::new();
    if dirty_only {
        for r in prev.iter_dirty_rows() {
            ids.insert(r);
        }
        for r in curr.iter_dirty_rows() {
            ids.insert(r);
        }
    } else {
        for r in prev.iter_active_rows() {
            ids.insert(r);
        }
        for r in curr.iter_active_rows() {
            ids.insert(r);
        }
    }
    ids.into_iter().filter(|&r| row_is_live(curr, r)).collect()
}

/// Pack Prev/Curr faces with identical `row_id` order (index-aligned for WGSL mix).
/// Skips ids that are TOMBSTONE on Curr.
///
/// ABA / freelist reuse: if Prev is missing, TOMBSTONE, or generation ≠ Curr,
/// Prev sample is copied from Curr (no ghost→new entity lerp). Matches x008e
/// Prev/Curr ABA contract.
pub fn pack_aligned_gpu_faces(
    prev: &EpochSheet,
    curr: &EpochSheet,
    row_ids: &[u32],
) -> (Vec<GpuEpochRow>, Vec<GpuEpochRow>) {
    let mut prev_rows = Vec::with_capacity(row_ids.len());
    let mut curr_rows = Vec::with_capacity(row_ids.len());
    for &row in row_ids {
        if !row_is_live(curr, row) {
            continue;
        }
        let ci = row as usize;
        let cxf = curr.world_transforms[ci];
        let curr_gen = curr.generation(row);
        let prev_usable = row_is_live(prev, row)
            && prev.generation(row) == curr_gen
            && (row as usize) < prev.world_transforms.len();
        let pxf = if prev_usable {
            prev.world_transforms[row as usize]
        } else {
            // Spawn / freelist reuse / tomb→live: hold at Curr (α-lerp no-op).
            cxf
        };
        prev_rows.push(GpuEpochRow::from_world(row, &pxf));
        curr_rows.push(GpuEpochRow::from_world(row, &cxf));
    }
    (prev_rows, curr_rows)
}

/// Single-sheet pack (debug / non-lerp). Prefer [`pack_aligned_gpu_faces`] for interpolate.
#[cfg_attr(
    not(test),
    allow(dead_code, reason = "unit-tested API; SceneGpu uses aligned faces")
)]
pub fn pack_gpu_rows(sheet: &EpochSheet, dirty_only: bool) -> Vec<GpuEpochRow> {
    let ids = aligned_row_ids(sheet, sheet, dirty_only);
    let (rows, _) = pack_aligned_gpu_faces(sheet, sheet, &ids);
    rows
}

/// Fill `out` in place (reuse allocation) from **packed** Curr face + `owner_scope`.
///
/// Pass rows from [`pack_aligned_gpu_faces`] only — raw `row_ids` that still contain
/// non-live entries would desync indices if enumerated directly.
pub fn fill_entity_pack_index(
    out: &mut HashMap<u32, u32>,
    curr: &EpochSheet,
    packed: &[GpuEpochRow],
) {
    out.clear();
    for (i, row) in packed.iter().enumerate() {
        let idx = row.row_id as usize;
        if let Some(&entity_id) = curr.owner_scope.get(idx) {
            out.insert(entity_id, i as u32);
        }
    }
}

/// CPU twin of [`INTERPOLATE_WGSL`] (hemisphere-aware nlerp). No GPU readback.
pub fn lerp_gpu_rows(prev: &GpuEpochRow, curr: &GpuEpochRow, alpha: f32) -> GpuEpochRow {
    let a = alpha.clamp(0.0, 1.0);
    let mut pr = prev.rot;
    let cr = curr.rot;
    let dot = pr[0] * cr[0] + pr[1] * cr[1] + pr[2] * cr[2] + pr[3] * cr[3];
    if dot < 0.0 {
        pr = [-pr[0], -pr[1], -pr[2], -pr[3]];
    }
    let mut rot = [
        pr[0] + (cr[0] - pr[0]) * a,
        pr[1] + (cr[1] - pr[1]) * a,
        pr[2] + (cr[2] - pr[2]) * a,
        pr[3] + (cr[3] - pr[3]) * a,
    ];
    let len = (rot[0] * rot[0] + rot[1] * rot[1] + rot[2] * rot[2] + rot[3] * rot[3]).sqrt();
    if len > 1e-12 {
        rot = [rot[0] / len, rot[1] / len, rot[2] / len, rot[3] / len];
    } else {
        rot = [0.0, 0.0, 0.0, 1.0];
    }
    let mut scale = [
        prev.scale[0] + (curr.scale[0] - prev.scale[0]) * a,
        prev.scale[1] + (curr.scale[1] - prev.scale[1]) * a,
        prev.scale[2] + (curr.scale[2] - prev.scale[2]) * a,
    ];
    if scale[0] * scale[0] + scale[1] * scale[1] + scale[2] * scale[2] < 1e-12 {
        scale = [1.0, 1.0, 1.0];
    }
    GpuEpochRow {
        pos: [
            prev.pos[0] + (curr.pos[0] - prev.pos[0]) * a,
            prev.pos[1] + (curr.pos[1] - prev.pos[1]) * a,
            prev.pos[2] + (curr.pos[2] - prev.pos[2]) * a,
        ],
        _pad0: 0.0,
        rot,
        scale,
        row_id: curr.row_id,
    }
}

/// Create / rewrite storage buffers for Prev/Curr/Out (appearance path only).
pub struct EpochGpuBuffers {
    pub prev: wgpu::Buffer,
    pub curr: wgpu::Buffer,
    pub out: wgpu::Buffer,
    pub params: wgpu::Buffer,
    pub capacity: u32,
}

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct InterpParams {
    alpha: f32,
    row_count: u32,
    _pad: [f32; 2],
}

impl EpochGpuBuffers {
    pub fn new(device: &wgpu::Device, capacity: u32) -> Self {
        let size = (capacity as u64) * std::mem::size_of::<GpuEpochRow>() as u64;
        let storage = |label: &str, usage: wgpu::BufferUsages| {
            device.create_buffer(&wgpu::BufferDescriptor {
                label: Some(label),
                size: size.max(std::mem::size_of::<GpuEpochRow>() as u64),
                usage,
                mapped_at_creation: false,
            })
        };
        Self {
            prev: storage(
                "epoch_prev",
                wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST,
            ),
            curr: storage(
                "epoch_curr",
                wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST,
            ),
            // STORAGE: compute write + vertex read (x0091). VERTEX kept for future MDI.
            out: storage(
                "epoch_out",
                wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::VERTEX,
            ),
            params: device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("epoch_interp_params"),
                size: std::mem::size_of::<InterpParams>() as u64,
                usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
                mapped_at_creation: false,
            }),
            capacity,
        }
    }

    /// Upload index-aligned Prev/Curr packs from [`pack_aligned_gpu_faces`]. No readback.
    ///
    /// Panics in debug if `prev.len() != curr.len()` (mismatched row correspondence).
    pub fn upload_faces(
        &self,
        queue: &wgpu::Queue,
        prev: &[GpuEpochRow],
        curr: &[GpuEpochRow],
        alpha: f32,
    ) {
        debug_assert_eq!(
            prev.len(),
            curr.len(),
            "Prev/Curr GPU packs must share the same aligned row_id list"
        );
        debug_assert!(
            prev.iter()
                .zip(curr.iter())
                .all(|(p, c)| p.row_id == c.row_id),
            "Prev/Curr GPU packs must match row_id at each index"
        );
        let n = prev.len().min(self.capacity as usize) as u32;
        if n > 0 {
            let end = n as usize;
            queue.write_buffer(&self.prev, 0, bytemuck::cast_slice(&prev[..end]));
            queue.write_buffer(&self.curr, 0, bytemuck::cast_slice(&curr[..end]));
        }
        let params = InterpParams {
            alpha: alpha.clamp(0.0, 1.0),
            row_count: n,
            _pad: [0.0; 2],
        };
        queue.write_buffer(&self.params, 0, bytemuck::bytes_of(&params));
    }
}

/// Build compute pipeline for [`INTERPOLATE_WGSL`].
pub fn create_interpolate_pipeline(
    device: &wgpu::Device,
) -> (wgpu::ComputePipeline, wgpu::BindGroupLayout) {
    let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
        label: Some("epoch_interpolate"),
        source: wgpu::ShaderSource::Wgsl(INTERPOLATE_WGSL.into()),
    });
    let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
        label: Some("epoch_interp_bgl"),
        entries: &[
            wgpu::BindGroupLayoutEntry {
                binding: 0,
                visibility: wgpu::ShaderStages::COMPUTE,
                ty: wgpu::BindingType::Buffer {
                    ty: wgpu::BufferBindingType::Storage { read_only: true },
                    has_dynamic_offset: false,
                    min_binding_size: None,
                },
                count: None,
            },
            wgpu::BindGroupLayoutEntry {
                binding: 1,
                visibility: wgpu::ShaderStages::COMPUTE,
                ty: wgpu::BindingType::Buffer {
                    ty: wgpu::BufferBindingType::Storage { read_only: true },
                    has_dynamic_offset: false,
                    min_binding_size: None,
                },
                count: None,
            },
            wgpu::BindGroupLayoutEntry {
                binding: 2,
                visibility: wgpu::ShaderStages::COMPUTE,
                ty: wgpu::BindingType::Buffer {
                    ty: wgpu::BufferBindingType::Storage { read_only: false },
                    has_dynamic_offset: false,
                    min_binding_size: None,
                },
                count: None,
            },
            wgpu::BindGroupLayoutEntry {
                binding: 3,
                visibility: wgpu::ShaderStages::COMPUTE,
                ty: wgpu::BindingType::Buffer {
                    ty: wgpu::BufferBindingType::Uniform,
                    has_dynamic_offset: false,
                    min_binding_size: None,
                },
                count: None,
            },
        ],
    });
    let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
        label: Some("epoch_interp_pl"),
        bind_group_layouts: &[&layout],
        push_constant_ranges: &[],
    });
    let pipeline = device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
        label: Some("epoch_interpolate_pipeline"),
        layout: Some(&pipeline_layout),
        module: &shader,
        entry_point: Some("interpolate"),
        compilation_options: Default::default(),
        cache: None,
    });
    (pipeline, layout)
}

pub fn make_interp_bind_group(
    device: &wgpu::Device,
    layout: &wgpu::BindGroupLayout,
    buffers: &EpochGpuBuffers,
) -> wgpu::BindGroup {
    device.create_bind_group(&wgpu::BindGroupDescriptor {
        label: Some("epoch_interp_bg"),
        layout,
        entries: &[
            wgpu::BindGroupEntry {
                binding: 0,
                resource: buffers.prev.as_entire_binding(),
            },
            wgpu::BindGroupEntry {
                binding: 1,
                resource: buffers.curr.as_entire_binding(),
            },
            wgpu::BindGroupEntry {
                binding: 2,
                resource: buffers.out.as_entire_binding(),
            },
            wgpu::BindGroupEntry {
                binding: 3,
                resource: buffers.params.as_entire_binding(),
            },
        ],
    })
}

/// Virtual-scroll Hierarchy: emit only the window slice (O(window + skipped), not full list).
#[allow(
    dead_code,
    reason = "S-28 API surface; used by tests until Hierarchy UI mounts"
)]
pub fn hierarchy_visible_rows(
    topo: &FlatTopology,
    collapsed: &HashSet<u32>,
    window_start: usize,
    window_len: usize,
) -> Vec<u32> {
    if window_len == 0 {
        return Vec::new();
    }
    let mut out = Vec::with_capacity(window_len);
    let mut visible_idx = 0usize;
    let mut i = 0usize;
    let order = &topo.euler_order;
    while i < order.len() && out.len() < window_len {
        let row = order[i];
        if visible_idx >= window_start {
            out.push(row);
        }
        visible_idx += 1;
        if collapsed.contains(&row) {
            if let Some(skip) = topo.collapsed_skip_order_index(row) {
                let tin = topo.euler_tin.get(row as usize).copied().unwrap_or(0) as usize;
                let jump = (skip as usize).saturating_sub(tin);
                i = i.saturating_add(jump.max(1));
                continue;
            }
        }
        i += 1;
    }
    out
}

/// Format Bitmask / Tape X-Ray panel text for System UI.
pub fn format_xray_panel(
    row_id: u32,
    prev: &EpochSheet,
    curr: &EpochSheet,
    topo: &FlatTopology,
    bounds: SceneBounds,
) -> String {
    let idx = row_id as usize;
    if idx >= curr.readiness.len() || idx >= curr.world_transforms.len() {
        return format!("row {row_id}: out of range");
    }
    let mask = curr.readiness[idx].0;
    let bits = format!("{mask:064b}");
    let curr_xf = &curr.world_transforms[idx];
    let prev_xf = prev.world_transforms.get(idx).copied().unwrap_or_default();
    let depth = depth_via_parents(topo, row_id);
    let center = [
        curr_xf.position[0] as f32,
        curr_xf.position[1] as f32,
        curr_xf.position[2] as f32,
    ];
    let morton = morton_code30(center, bounds);
    format!(
        "X-Ray row {row_id}\n\
         mask {bits}\n\
         depth {depth}  morton 0x{morton:08x}\n\
         prev ({:.3},{:.3},{:.3})\n\
         curr ({:.3},{:.3},{:.3})",
        prev_xf.position[0],
        prev_xf.position[1],
        prev_xf.position[2],
        curr_xf.position[0],
        curr_xf.position[1],
        curr_xf.position[2],
    )
}

fn depth_via_parents(topo: &FlatTopology, row_id: u32) -> u32 {
    let mut depth = 0u32;
    let mut cur = row_id;
    let mut guard = 0usize;
    while let Some(&parent) = topo.parent_of.get(cur as usize) {
        if parent == u32::MAX {
            break;
        }
        cur = parent;
        depth = depth.saturating_add(1);
        guard += 1;
        if guard > topo.parent_of.len() {
            break;
        }
    }
    depth
}

/// If no X-Ray row is selected, pick the first active Curr row (explicit ensure).
pub fn ensure_xray_row(state: &EpochViewState) -> Option<u32> {
    if let Some(r) = state.xray_row() {
        return Some(r);
    }
    let first = state.snapshots.load_curr().iter_active_rows().next()?;
    state.set_xray_row(Some(first));
    Some(first)
}

/// Snapshot helper used by dash / debug overlay (read-only; does not change selection).
pub fn xray_text_from_state(state: &EpochViewState) -> Option<String> {
    let row = state.xray_row()?;
    let faces = state.snapshots.load_pair();
    let topo = state.topology.lock().ok()?;
    let bounds = state.scene_bounds.lock().map(|g| *g).unwrap_or_default();
    Some(format_xray_panel(
        row,
        &faces.prev,
        &faces.curr,
        &topo,
        bounds,
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use alchemy_shared_sim::tape::WorldTransformf64;
    use alchemy_shared_sim::topology::FlatTopology;

    #[test]
    fn hierarchy_collapse_skips_subtree() {
        let topo = FlatTopology::from_parents(vec![u32::MAX, 0, 1, 0]).unwrap();
        let mut collapsed = HashSet::new();
        collapsed.insert(1);
        let vis = hierarchy_visible_rows(&topo, &collapsed, 0, 16);
        // 0, then 1 (collapsed → skip 2), then 3
        assert_eq!(vis, vec![0, 1, 3]);
    }

    #[test]
    fn xray_formats_mask_and_morton() {
        let mut curr = EpochSheet::with_capacity(1);
        let h = curr.alloc_row();
        curr.set_transform(h.row_id, WorldTransformf64::identity_at([1.0, 2.0, 3.0]))
            .unwrap();
        curr.readiness[0] = ReadinessMask(ReadinessMask::ALLOCATED | ReadinessMask::COLLIDER);
        let prev = curr.clone();
        let topo = FlatTopology::from_parents(vec![u32::MAX]).unwrap();
        let text = format_xray_panel(0, &prev, &curr, &topo, SceneBounds::default());
        assert!(text.contains("X-Ray row 0"));
        assert!(text.contains("morton"));
        assert!(text.contains("curr (1.000,2.000,3.000)"));
    }

    #[test]
    fn pack_gpu_rows_skips_tombstone() {
        let mut sheet = EpochSheet::with_capacity(2);
        let a = sheet.alloc_row();
        let b = sheet.alloc_row();
        sheet.tombstone(b).unwrap();
        let rows = pack_gpu_rows(&sheet, false);
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].row_id, a.row_id);
    }

    #[test]
    fn aligned_faces_share_row_ids_even_if_dirty_sets_differ() {
        let mut prev = EpochSheet::with_capacity(3);
        let mut curr = EpochSheet::with_capacity(3);
        let a = prev.alloc_row();
        let b = prev.alloc_row();
        let _ = curr.alloc_row();
        let _ = curr.alloc_row();
        prev.clear_dirty();
        curr.clear_dirty();
        prev.mark_dirty(a.row_id);
        curr.mark_dirty(b.row_id);
        let ids = aligned_row_ids(&prev, &curr, true);
        assert_eq!(ids, vec![a.row_id, b.row_id]);
        let (p, c) = pack_aligned_gpu_faces(&prev, &curr, &ids);
        assert_eq!(p.len(), c.len());
        assert!(p.iter().zip(c.iter()).all(|(x, y)| x.row_id == y.row_id));
    }

    #[test]
    fn interpolate_wgsl_is_compute_entry() {
        assert!(INTERPOLATE_WGSL.contains("@compute"));
        assert!(INTERPOLATE_WGSL.contains("fn interpolate"));
        assert!(INTERPOLATE_WGSL.contains("dot(pr, c.rot)"));
        assert_eq!(std::mem::size_of::<GpuEpochRow>() % 16, 0);
    }

    #[test]
    fn pack_index_override_sentinel_is_max() {
        assert_eq!(PACK_INDEX_MODEL_OVERRIDE, u32::MAX);
    }

    #[test]
    fn fill_entity_pack_index_maps_owner_scope() {
        let mut sheet = EpochSheet::with_capacity(2);
        let a = sheet.alloc_row();
        let b = sheet.alloc_row();
        sheet.owner_scope[a.row_id as usize] = 10;
        sheet.owner_scope[b.row_id as usize] = 20;
        let ids = vec![a.row_id, b.row_id];
        let (_p, c) = pack_aligned_gpu_faces(&sheet, &sheet, &ids);
        let mut map = HashMap::new();
        fill_entity_pack_index(&mut map, &sheet, &c);
        assert_eq!(map.get(&10), Some(&0));
        assert_eq!(map.get(&20), Some(&1));
    }

    #[test]
    fn fill_entity_pack_index_ignores_non_live_skipped_by_pack() {
        let mut sheet = EpochSheet::with_capacity(2);
        let a = sheet.alloc_row();
        let b = sheet.alloc_row();
        sheet.owner_scope[a.row_id as usize] = 10;
        sheet.owner_scope[b.row_id as usize] = 20;
        sheet.tombstone(b).unwrap();
        // Caller mistakenly passes tombstoned id; pack skips it — map must not shift.
        let ids = vec![a.row_id, b.row_id];
        let (_p, c) = pack_aligned_gpu_faces(&sheet, &sheet, &ids);
        assert_eq!(c.len(), 1);
        let mut map = HashMap::new();
        fill_entity_pack_index(&mut map, &sheet, &c);
        assert_eq!(map.get(&10), Some(&0));
        assert!(!map.contains_key(&20));
    }

    #[test]
    fn lerp_gpu_rows_matches_midpoint_translation() {
        let prev = GpuEpochRow {
            pos: [0.0, 0.0, 0.0],
            rot: [0.0, 0.0, 0.0, 1.0],
            scale: [1.0, 1.0, 1.0],
            row_id: 1,
            ..Default::default()
        };
        let curr = GpuEpochRow {
            pos: [2.0, 4.0, 6.0],
            rot: [0.0, 0.0, 0.0, 1.0],
            scale: [1.0, 1.0, 1.0],
            row_id: 1,
            ..Default::default()
        };
        let mid = lerp_gpu_rows(&prev, &curr, 0.5);
        assert!((mid.pos[0] - 1.0).abs() < 1e-5);
        assert!((mid.pos[1] - 2.0).abs() < 1e-5);
        assert!((mid.pos[2] - 3.0).abs() < 1e-5);
    }

    #[test]
    fn lerp_gpu_rows_hemisphere_flip_takes_short_arc() {
        // 180° apart on w axis with opposite signs → without flip, mix goes through zero.
        let prev = GpuEpochRow {
            rot: [0.0, 0.0, 0.0, 1.0],
            scale: [1.0, 1.0, 1.0],
            row_id: 1,
            ..Default::default()
        };
        let curr = GpuEpochRow {
            rot: [0.0, 0.0, 0.0, -1.0],
            scale: [1.0, 1.0, 1.0],
            row_id: 1,
            ..Default::default()
        };
        let mid = lerp_gpu_rows(&prev, &curr, 0.5);
        let len = (mid.rot[0] * mid.rot[0]
            + mid.rot[1] * mid.rot[1]
            + mid.rot[2] * mid.rot[2]
            + mid.rot[3] * mid.rot[3])
            .sqrt();
        assert!((len - 1.0).abs() < 1e-4, "len={len}");
        // After flip, curr ≈ +identity → mid stays near identity (not zero quat).
        assert!(mid.rot[3].abs() > 0.9, "rot={:?}", mid.rot);
    }

    #[test]
    fn build_display_frame_uses_faces_published_at_for_alpha() {
        let mut curr = EpochSheet::with_capacity(1);
        let h = curr.alloc_row();
        curr.owner_scope[h.row_id as usize] = 42;
        curr.set_transform(h.row_id, WorldTransformf64::identity_at([1.0, 0.0, 0.0]))
            .unwrap();
        let prev = curr.clone();
        let start = Instant::now();
        let faces = Arc::new(EpochFaces {
            prev: Arc::new(prev),
            curr: Arc::new(curr),
            published_at_ns: 1, // published_instant(1) ≈ process_start + 1ns
        });
        let frame = build_display_frame(faces, start);
        assert!(frame.poses.contains_key(&42));
        assert_eq!(frame.prev_rows.len(), frame.curr_rows.len());
        // α from faces.published_at_ns, not a second load_pair.
        assert!((0.0..=1.0).contains(&frame.alpha));
    }

    #[test]
    fn pack_aba_reuse_copies_curr_into_prev_face() {
        // Prev: live ghost at row 0. Curr: tombstone then freelist-reuse same row_id.
        let mut prev = EpochSheet::with_capacity(1);
        let old = prev.alloc_row();
        prev.set_transform(old.row_id, WorldTransformf64::identity_at([9.0, 0.0, 0.0]))
            .unwrap();
        let mut curr = prev.clone();
        curr.tombstone(old).unwrap();
        let neu = curr.alloc_row();
        assert_eq!(neu.row_id, old.row_id, "freelist must reuse row_id");
        assert_ne!(neu.generation, old.generation);
        curr.set_transform(neu.row_id, WorldTransformf64::identity_at([1.0, 0.0, 0.0]))
            .unwrap();
        let ids = aligned_row_ids(&prev, &curr, false);
        let (p, c) = pack_aligned_gpu_faces(&prev, &curr, &ids);
        assert_eq!(p.len(), 1);
        assert_eq!(c[0].pos[0], 1.0);
        // Must not lerp from ghost 9.0 → 1.0.
        assert_eq!(p[0].pos[0], 1.0);
    }
}
