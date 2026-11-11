//! Client-side Frame → EpochSnapshotPair kinematic mirror (x0090).
//!
//! Guarantees: after a verified Applied Frame, published Prev = previous Curr and
//! Curr reflects kinematic TRS (`inv_mass=0`, no `DYNAMIC`); membership follows the
//! full entity map; freelist row reuse is safe for GPU pack via generation/ABA gate
//! in [`crate::epoch_view::pack_aligned_gpu_faces`].
//! Non-guarantees: topology / hierarchy; color on tape; host ArcSwap sharing;
//! XPBD; draw authority (stays on `EntityPose`).

use std::collections::HashMap;
use std::sync::Arc;
use std::time::{Duration, Instant};

use alchemy_shared_sim::{EpochSheet, NodeHandle, ReadinessMask, WorldTransformf64};

use crate::epoch_view::EpochViewState;

/// Wire `RenderFrame` publish period for SceneGpu α-lerp.
///
/// Host physics ticks at 60Hz; **signed Frame / snapshot on the wire is 30Hz**
/// (`x0006` 内部 60Hz / 配信 30Hz). α tracks wire Frame spacing, not Rapier tick.
pub const EPOCH_FRAME_PERIOD: Duration = Duration::from_nanos(33_333_333);

/// Thin TRS for mirror ingest (avoids `epoch_mirror → scene` module cycle).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct KinematicPose {
    pub x: f32,
    pub y: f32,
    pub z: f32,
    pub qx: f32,
    pub qy: f32,
    pub qz: f32,
    pub qw: f32,
    pub sx: f32,
    pub sy: f32,
    pub sz: f32,
}

impl Default for KinematicPose {
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
        }
    }
}

/// Working tape + `entity_id →` row map under [`EpochViewState`].
///
/// `live` is `Arc` so publish can share the Curr pointer (`publish_arcs`) and
/// keep dirty bits on the published face while clearing only the working copy.
/// Steady ingest still does **two** deep sheet clones (`make_mut` for mutate +
/// `make_mut` for `clear_dirty` while faces hold Curr) — not a clone-count win.
#[derive(Debug)]
pub struct EpochFrameMirror {
    entity_rows: HashMap<u32, NodeHandle>,
    live: Arc<EpochSheet>,
    pub publishes: u64,
}

impl Default for EpochFrameMirror {
    fn default() -> Self {
        Self {
            entity_rows: HashMap::new(),
            live: Arc::new(EpochSheet::default()),
            publishes: 0,
        }
    }
}

impl EpochFrameMirror {
    pub fn entity_count(&self) -> usize {
        self.entity_rows.len()
    }

    pub fn row_for(&self, entity_id: u32) -> Option<NodeHandle> {
        self.entity_rows.get(&entity_id).copied()
    }

    /// Reconcile `entities` (full WorldView table after apply) into live Curr and publish.
    pub fn ingest_world(
        &mut self,
        snapshots: &alchemy_shared_sim::EpochSnapshotPair,
        entities: &HashMap<u32, KinematicPose>,
    ) {
        let prev = Arc::clone(&self.live);
        {
            let sheet = Arc::make_mut(&mut self.live);

            let stale: Vec<u32> = self
                .entity_rows
                .keys()
                .copied()
                .filter(|id| !entities.contains_key(id))
                .collect();
            for id in stale {
                if let Some(handle) = self.entity_rows.remove(&id) {
                    let _ = sheet.tombstone(handle);
                }
            }

            for (&entity_id, pose) in entities {
                let handle = match self.entity_rows.get(&entity_id).copied() {
                    Some(h) => h,
                    None => {
                        let h = sheet.alloc_row();
                        let idx = h.row_id as usize;
                        sheet.readiness[idx] =
                            ReadinessMask(ReadinessMask::ALLOCATED | ReadinessMask::TRANSFORM);
                        sheet.owner_scope[idx] = entity_id;
                        self.entity_rows.insert(entity_id, h);
                        h
                    }
                };
                let xf = pose_to_kinematic(pose);
                if let Err(err) = sheet.set_transform(handle.row_id, xf) {
                    log::warn!("epoch mirror set_transform entity_id={entity_id}: {err:?}");
                }
            }
        }

        // `0` means unset on EpochFaces; never publish a real clock as 0.
        let published_at_ns = monotonic_ns().max(1);
        // Publish before clear_dirty so the Curr Arc keeps this tick's dirty bits.
        snapshots.publish_arcs(prev, Arc::clone(&self.live), published_at_ns);
        // CoW again if faces still share `live` — clear only the working copy.
        Arc::make_mut(&mut self.live).clear_dirty();
        self.publishes = self.publishes.wrapping_add(1);
    }
}

fn pose_to_kinematic(pose: &KinematicPose) -> WorldTransformf64 {
    let q = glam::Quat::from_xyzw(pose.qx, pose.qy, pose.qz, pose.qw);
    let rot = if q.length_squared() < 1e-12 {
        glam::Quat::IDENTITY
    } else {
        q.normalize()
    };
    let scale = {
        let s = glam::Vec3::new(pose.sx, pose.sy, pose.sz);
        if s.length_squared() < 1e-12 {
            glam::Vec3::ONE
        } else {
            s
        }
    };
    WorldTransformf64 {
        position: [pose.x as f64, pose.y as f64, pose.z as f64],
        inv_mass: 0.0,
        rotation: [rot.x, rot.y, rot.z, rot.w],
        scale: [scale.x, scale.y, scale.z],
        frame_delta_xyz: [0.0, 0.0, 0.0],
        flags: 0,
        _pad0: 0,
        _pad1: [0; 4],
    }
}

fn process_start() -> Instant {
    static START: std::sync::OnceLock<Instant> = std::sync::OnceLock::new();
    *START.get_or_init(Instant::now)
}

fn monotonic_ns() -> u64 {
    process_start()
        .elapsed()
        .as_nanos()
        .min(u128::from(u64::MAX)) as u64
}

/// Decode `EpochFaces::published_at_ns` using the shared process-start clock.
pub fn published_instant(ns: u64) -> Option<Instant> {
    if ns == 0 {
        None
    } else {
        Some(process_start() + Duration::from_nanos(ns))
    }
}

/// α in \[0,1\] from last mirror publish vs assumed **wire** 30Hz period.
pub fn epoch_interp_alpha(published_at: Option<Instant>, now: Instant) -> f32 {
    let Some(at) = published_at else {
        return 1.0;
    };
    let elapsed = now.saturating_duration_since(at);
    let period = EPOCH_FRAME_PERIOD.as_secs_f32().max(1e-6);
    (elapsed.as_secs_f32() / period).clamp(0.0, 1.0)
}

/// Ingest after WorldView mutation (Applied Frame).
pub fn ingest_applied_world(epoch: &EpochViewState, entities: &HashMap<u32, KinematicPose>) {
    let Ok(mut mirror) = epoch.mirror.lock() else {
        log::warn!("epoch mirror lock poisoned; skip ingest");
        return;
    };
    mirror.ingest_world(&epoch.snapshots, entities);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::epoch_view::{aligned_row_ids, pack_aligned_gpu_faces};
    use alchemy_shared_sim::ReadinessMask;

    fn pose_at(x: f32) -> KinematicPose {
        KinematicPose {
            x,
            ..Default::default()
        }
    }

    #[test]
    fn consecutive_ingest_keeps_prev_curr_distinct() {
        let epoch = EpochViewState::new();
        let mut map = HashMap::new();
        map.insert(1, pose_at(0.0));
        ingest_applied_world(&epoch, &map);
        map.insert(1, pose_at(5.0));
        ingest_applied_world(&epoch, &map);

        let faces = epoch.snapshots.load_pair();
        assert!(faces.published_at_ns > 0);
        let row = epoch.mirror.lock().unwrap().row_for(1).unwrap().row_id as usize;
        assert_eq!(faces.prev.world_transforms[row].position[0], 0.0);
        assert_eq!(faces.curr.world_transforms[row].position[0], 5.0);
        assert_eq!(faces.curr.world_transforms[row].inv_mass, 0.0);
        assert!(!faces.curr.readiness[row].contains(ReadinessMask::DYNAMIC));
    }

    #[test]
    fn replace_clears_absent_entity_from_published_curr() {
        let epoch = EpochViewState::new();
        let mut map = HashMap::new();
        map.insert(1, pose_at(1.0));
        map.insert(2, pose_at(2.0));
        ingest_applied_world(&epoch, &map);

        map.remove(&1);
        ingest_applied_world(&epoch, &map);

        let curr = epoch.snapshots.load_curr();
        let mirror = epoch.mirror.lock().unwrap();
        assert!(mirror.row_for(1).is_none());
        let row2 = mirror.row_for(2).unwrap().row_id as usize;
        assert_eq!(curr.world_transforms[row2].position[0], 2.0);
        let active: Vec<u32> = curr.iter_active_rows().collect();
        assert_eq!(active, vec![row2 as u32]);
    }

    #[test]
    fn remove_tombs_and_drops_map_entry() {
        let epoch = EpochViewState::new();
        let mut map = HashMap::new();
        map.insert(7, pose_at(3.0));
        ingest_applied_world(&epoch, &map);
        let row = epoch.mirror.lock().unwrap().row_for(7).unwrap().row_id;

        map.clear();
        ingest_applied_world(&epoch, &map);

        let curr = epoch.snapshots.load_curr();
        assert!(epoch.mirror.lock().unwrap().row_for(7).is_none());
        assert!(curr.readiness[row as usize].contains(ReadinessMask::TOMBSTONE));
        assert_eq!(curr.iter_active_rows().count(), 0);
    }

    #[test]
    fn freelist_reuse_pack_does_not_lerp_ghost_pose() {
        let epoch = EpochViewState::new();
        let mut map = HashMap::new();
        map.insert(1, pose_at(9.0));
        ingest_applied_world(&epoch, &map);
        let row_old = epoch.mirror.lock().unwrap().row_for(1).unwrap();

        map.clear();
        ingest_applied_world(&epoch, &map);
        map.insert(2, pose_at(1.0));
        ingest_applied_world(&epoch, &map);
        let row_new = epoch.mirror.lock().unwrap().row_for(2).unwrap();
        assert_eq!(row_new.row_id, row_old.row_id, "expect freelist reuse");
        assert_ne!(row_new.generation, row_old.generation);

        let faces = epoch.snapshots.load_pair();
        let ids = aligned_row_ids(&faces.prev, &faces.curr, false);
        let (p, c) = pack_aligned_gpu_faces(&faces.prev, &faces.curr, &ids);
        assert_eq!(c.len(), 1);
        assert_eq!(c[0].pos[0], 1.0);
        assert_eq!(
            p[0].pos[0], 1.0,
            "ABA reuse must not α-lerp from tombstoned ghost 9.0"
        );
    }

    #[test]
    fn interp_alpha_clamps_to_unit_interval() {
        let t0 = Instant::now();
        assert!((epoch_interp_alpha(None, t0) - 1.0).abs() < f32::EPSILON);
        assert!((epoch_interp_alpha(Some(t0), t0) - 0.0).abs() < 1e-5);
        let later = t0 + EPOCH_FRAME_PERIOD + EPOCH_FRAME_PERIOD;
        assert!((epoch_interp_alpha(Some(t0), later) - 1.0).abs() < 1e-5);
    }
}
