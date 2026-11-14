//! Client Frame → [`PublishedPair`] kinematic mirror (x00aa).
//!
//! Guarantees: after a verified Applied Frame, prev/curr faces advance with `sim_tick`;
//! entity poses land in a single tip [`HotPhysicsPage`] (≤ [`ENTITIES_PER_PAGE`]);
//! NaN/Inf rejected via [`HotPhysicsPage::try_set_kinematic_pose`].
//! Non-guarantees: multi-page Directory / cell cross (Phase 3); host `MvccPageArena`
//! authority; GPU upload of page SoA (x00ac); replacing Epoch draw path this slice.
//!
//! **Capacity contract:** entities beyond [`ENTITIES_PER_PAGE`] are **dropped from the
//! MVCC tip only** (logged); Epoch mirror / WorldView still keep them. Draw sets can
//! diverge until Phase 3 multi-page Directory. Freelist locals are sparse — dense
//! `entity_count` WGSL dispatch must compact or skip inactive slots when wired.

use std::collections::HashMap;

use alchemy_shared_sim::mvcc::{
    HotPhysicsPage, KinematicPoseF32, PublishedFace, PublishedPair, ENTITIES_PER_PAGE,
};

use crate::epoch_mirror::KinematicPose;

/// Working entity→local map + ArcSwap publish pair for sim-tick interpolation.
pub struct MvccFrameMirror {
    entity_local: HashMap<u32, u16>,
    free_locals: Vec<u16>,
    next_local: u16,
    pair: PublishedPair,
    pub publishes: u64,
}

impl Default for MvccFrameMirror {
    fn default() -> Self {
        Self::new()
    }
}

impl MvccFrameMirror {
    pub fn new() -> Self {
        Self {
            entity_local: HashMap::new(),
            free_locals: Vec::new(),
            next_local: 0,
            pair: PublishedPair::new(),
            publishes: 0,
        }
    }

    pub fn entity_count(&self) -> usize {
        self.entity_local.len()
    }

    pub fn local_for(&self, entity_id: u32) -> Option<u16> {
        self.entity_local.get(&entity_id).copied()
    }

    pub fn load_prev(&self) -> std::sync::Arc<PublishedFace> {
        self.pair.load_prev()
    }

    pub fn load_curr(&self) -> std::sync::Arc<PublishedFace> {
        self.pair.load_curr()
    }

    /// Reconcile `entities` into a new tip page and publish (`sim_tick` = wire tick until Phase 3).
    pub fn ingest_world(&mut self, entities: &HashMap<u32, KinematicPose>, sim_tick: u64) {
        let mut page = HotPhysicsPage::zeroed_page();

        let stale: Vec<u32> = self
            .entity_local
            .keys()
            .copied()
            .filter(|id| !entities.contains_key(id))
            .collect();
        for id in stale {
            if let Some(local) = self.entity_local.remove(&id) {
                self.free_locals.push(local);
            }
        }

        for (&entity_id, pose) in entities {
            let local = match self.entity_local.get(&entity_id).copied() {
                Some(l) => l,
                None => match self.alloc_local() {
                    Some(l) => {
                        self.entity_local.insert(entity_id, l);
                        l
                    }
                    None => {
                        log::warn!(
                            "mvcc mirror: tip page full, drop entity_id={entity_id} (cap={ENTITIES_PER_PAGE})"
                        );
                        continue;
                    }
                },
            };
            let idx = local as usize;
            page.set_active(idx, true);
            let kp = KinematicPoseF32::xyz_quat(
                pose.x, pose.y, pose.z, pose.qx, pose.qy, pose.qz, pose.qw,
            );
            if !page.try_set_kinematic_pose(idx, kp) {
                page.quarantine_slot(idx);
                log::warn!("mvcc mirror: quarantine non-finite entity_id={entity_id}");
            }
        }

        self.pair.publish(PublishedFace { sim_tick, page });
        self.publishes = self.publishes.wrapping_add(1);
    }

    fn alloc_local(&mut self) -> Option<u16> {
        if let Some(l) = self.free_locals.pop() {
            return Some(l);
        }
        if (self.next_local as usize) >= ENTITIES_PER_PAGE {
            return None;
        }
        let l = self.next_local;
        self.next_local = self.next_local.saturating_add(1);
        Some(l)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pose_at(x: f32) -> KinematicPose {
        KinematicPose {
            x,
            ..Default::default()
        }
    }

    #[test]
    fn consecutive_ingest_advances_sim_tick_and_prev_curr() {
        let mut mirror = MvccFrameMirror::new();
        let mut map = HashMap::new();
        map.insert(1, pose_at(0.0));
        mirror.ingest_world(&map, 10);
        map.insert(1, pose_at(5.0));
        mirror.ingest_world(&map, 11);

        assert_eq!(mirror.load_curr().sim_tick, 11);
        assert_eq!(mirror.load_prev().sim_tick, 10);
        let local = mirror.local_for(1).unwrap() as usize;
        assert_eq!(mirror.load_prev().page.soa.pos_x[local], 0.0);
        assert_eq!(mirror.load_curr().page.soa.pos_x[local], 5.0);
        assert!(mirror.load_curr().page.is_active(local));
    }

    #[test]
    fn stale_entity_frees_local() {
        let mut mirror = MvccFrameMirror::new();
        let mut map = HashMap::new();
        map.insert(1, pose_at(1.0));
        mirror.ingest_world(&map, 1);
        let local = mirror.local_for(1).unwrap();
        map.clear();
        map.insert(2, pose_at(2.0));
        mirror.ingest_world(&map, 2);
        assert!(mirror.local_for(1).is_none());
        assert_eq!(mirror.local_for(2), Some(local)); // freelist reuse
        assert!(mirror.load_curr().page.is_active(local as usize));
        assert_eq!(mirror.load_curr().page.soa.pos_x[local as usize], 2.0);
    }
}
