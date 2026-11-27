//! AvatarRigBuffer — local secondary-bone XPBD outside Scene Store slots (x00e8).
//!
//! Guarantees: continuous SoA [`AvatarRigBuffer`] for tip / sway bones; soft
//! distance XPBD via [`step`]; spatial sync anchors limited to
//! Root / Head / Hands (+ few full-body extras) as [`LocalSlotId`] refs only;
//! secondary bones never call Scene Store `AllocateSlot` / SparseSet insert
//! ([`AvatarRigBuffer`] has no Scene Store / Arena API surface).
//! Non-guarantees: host 60Hz wiring; GPU skinning; full-body IK UX; rapier3d;
//! Scale / CCU KPI; RigidSolver (x00e9); EpochSheet / Grab / LBVH reuse.
//!
//! # Ring 2 placement
//!
//! Call [`step`] from Ring 2 **Step 2** (physics). This slice leaves host /
//! client tick **unwired** — unit tests exercise the minimal path.
//!
//! # Slot vs Rig boundary (§4.3)
//!
//! | Kind | Where |
//! |:---|:---|
//! | Root / Head / LeftHand / RightHand (+ few FB) | Scene Store + Sim Arena slots |
//! | Tip / sway secondary bones (100–150 / avatar) | [`AvatarRigBuffer`] only |

#![forbid(unsafe_code)]

mod buffer;
mod xpbd_step;

pub use buffer::{
    AvatarAnchorKind, AvatarAnchorWorld, AvatarRigBuffer, AvatarSpatialAnchors, BONE_PINNED,
    MAX_EXTRA_FULLBODY_ANCHORS,
};
pub use xpbd_step::step;

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ids::LocalSlotId;
    use crate::scene_store::{ApplyOutcome, CrdtOp, OpTimestamp, SceneStore};
    use crate::sim_arena::SimArena;

    fn ts(lamport: u64) -> OpTimestamp {
        OpTimestamp::new(lamport, 1)
    }

    fn spawn_anchor(store: &mut SceneStore, lamport: u64, node: u32) {
        assert_eq!(
            store.apply(CrdtOp::SpawnNode {
                ts: ts(lamport),
                node,
                parent: None,
            }),
            ApplyOutcome::Applied
        );
    }

    #[test]
    fn secondary_bones_do_not_allocate_scene_slots() {
        // §4.3: grow 128 sway bones while only anchor Spawns touch Scene Store.
        let mut store = SceneStore::new();
        spawn_anchor(&mut store, 1, 1); // Root
        spawn_anchor(&mut store, 2, 2); // Head
        spawn_anchor(&mut store, 3, 3); // LeftHand
        spawn_anchor(&mut store, 4, 4); // RightHand
        let mut arena = SimArena::with_slots(16);
        // Drain only the anchor Allocate patches — baseline after anchors exist.
        let patches: Vec<_> = store.drain_patches();
        assert_eq!(patches.len(), 4, "only four anchor Allocates expected");
        arena.step0_carry_and_apply_patches(&patches);
        let store_len = store.len();
        let arena_slots = arena.slot_count();
        assert!(store.pending_patches().is_empty());

        let mut anchors = AvatarSpatialAnchors::default();
        assert!(anchors.set_primary(AvatarAnchorKind::Root, Some(1 as LocalSlotId)));
        assert!(anchors.set_primary(AvatarAnchorKind::Head, Some(2)));
        assert!(anchors.set_primary(AvatarAnchorKind::LeftHand, Some(3)));
        assert!(anchors.set_primary(AvatarAnchorKind::RightHand, Some(4)));
        assert_eq!(anchors.spatial_slot_count(), 4);

        let mut rig = AvatarRigBuffer::with_capacity(128);
        let pinned = rig
            .push_pinned(AvatarAnchorKind::Head, [0.0, 1.6, 0.0])
            .expect("pinned");
        assert_eq!(pinned, 0);
        for i in 1..128u32 {
            let y = 1.6 - 0.05 * i as f32;
            rig.push_secondary(i - 1, 0.05, 1e-4, [0.0, y, 0.0])
                .expect("secondary");
        }
        assert_eq!(rig.bone_count(), 128);
        assert_eq!(
            anchors.spatial_slot_count(),
            4,
            "spatial refs stay at anchors only"
        );

        // Rig construction must not enqueue Scene Store patches or grow Arena.
        assert!(
            store.pending_patches().is_empty(),
            "AvatarRigBuffer must not enqueue SimPatch"
        );
        assert_eq!(store.len(), store_len);
        assert_eq!(arena.slot_count(), arena_slots);
        // Second drain stays empty — no latent Allocates from bone growth.
        assert!(store.drain_patches().is_empty());
    }

    #[test]
    fn soft_chain_xpbd_moves_under_gravity() {
        let mut rig = AvatarRigBuffer::with_capacity(3);
        rig.push_pinned(AvatarAnchorKind::Root, [0.0, 0.0, 0.0])
            .unwrap();
        rig.push_secondary(0, 1.0, 0.0, [0.0, -1.0, 0.0])
            .unwrap();
        rig.push_secondary(1, 1.0, 1e-3, [0.0, -1.0, 0.0])
            .unwrap();

        let mut world = AvatarAnchorWorld::default();
        assert!(world.set(AvatarAnchorKind::Root, [0.0, 0.0, 0.0]));

        let tip_y0 = rig.pos_y(2);
        let dt = 1.0 / 60.0;
        for _ in 0..90 {
            step(&mut rig, &world, dt, 4, [0.0, -9.81, 0.0]);
        }
        let mid = [rig.pos_x(1), rig.pos_y(1), rig.pos_z(1)];
        let tip = [rig.pos_x(2), rig.pos_y(2), rig.pos_z(2)];
        assert!(mid.iter().all(|v| v.is_finite()), "mid finite: {mid:?}");
        assert!(tip.iter().all(|v| v.is_finite()), "tip finite: {tip:?}");
        assert!(
            tip[1] < tip_y0 - 0.5,
            "soft tip should drop under gravity (y0={tip_y0}, y={})",
            tip[1]
        );
        let dist_mid_pin = (mid[0].powi(2) + mid[1].powi(2) + mid[2].powi(2)).sqrt();
        assert!(
            (dist_mid_pin - 1.0).abs() < 0.1,
            "hard mid near rest=1 (dist={dist_mid_pin})"
        );
        let dist_tip_mid = ((tip[0] - mid[0]).powi(2)
            + (tip[1] - mid[1]).powi(2)
            + (tip[2] - mid[2]).powi(2))
        .sqrt();
        assert!(
            (dist_tip_mid - 1.0).abs() < 0.2,
            "soft tip near rest=1 (dist={dist_tip_mid})"
        );
    }

    #[test]
    fn pin_bone_snaps_to_anchor_each_step() {
        let mut rig = AvatarRigBuffer::with_capacity(1);
        rig.push_pinned(AvatarAnchorKind::Head, [0.0, 0.0, 0.0])
            .unwrap();
        let mut world = AvatarAnchorWorld::default();
        assert!(world.set(AvatarAnchorKind::Head, [1.0, 2.0, 3.0]));
        step(&mut rig, &world, 1.0 / 60.0, 2, [0.0, -9.81, 0.0]);
        assert_eq!([rig.pos_x(0), rig.pos_y(0), rig.pos_z(0)], [1.0, 2.0, 3.0]);
    }

    #[test]
    fn extra_discriminant_round_trips_in_range() {
        for i in 0..MAX_EXTRA_FULLBODY_ANCHORS as u8 {
            let kind = AvatarAnchorKind::Extra(i);
            let d = kind.try_discriminant().expect("in-range Extra");
            assert_eq!(AvatarAnchorKind::from_discriminant(d), Some(kind));
        }
        assert_eq!(AvatarAnchorKind::Root.try_discriminant(), Some(0));
        assert_eq!(
            AvatarAnchorKind::from_discriminant(0),
            Some(AvatarAnchorKind::Root)
        );
    }

    #[test]
    fn out_of_range_extra_rejected() {
        assert!(AvatarAnchorKind::Extra(8).try_discriminant().is_none());
        assert!(AvatarAnchorKind::Extra(255).try_discriminant().is_none());
        // Old bug: `4u8 + 252` wrapped to Root (0). Encoding must refuse Extra(252).
        assert_eq!(4u8.wrapping_add(252), 0);
        assert!(AvatarAnchorKind::Extra(252).try_discriminant().is_none());

        let mut rig = AvatarRigBuffer::with_capacity(1);
        assert!(rig
            .push_pinned(AvatarAnchorKind::Extra(8), [0.0, 0.0, 0.0])
            .is_none());
        assert!(rig
            .push_pinned(AvatarAnchorKind::Extra(255), [1.0, 0.0, 0.0])
            .is_none());
        assert_eq!(rig.bone_count(), 0);

        let mut anchors = AvatarSpatialAnchors::default();
        assert!(!anchors.set_primary(AvatarAnchorKind::Extra(0), Some(9)));
        for _ in 0..MAX_EXTRA_FULLBODY_ANCHORS {
            assert!(anchors.push_extra(10));
        }
        assert!(!anchors.push_extra(11));
        assert_eq!(anchors.spatial_slot_count(), MAX_EXTRA_FULLBODY_ANCHORS);
    }

    #[test]
    fn push_secondary_rejects_bad_parent_and_non_finite() {
        let mut rig = AvatarRigBuffer::with_capacity(2);
        rig.push_pinned(AvatarAnchorKind::Root, [0.0, 0.0, 0.0])
            .unwrap();
        assert!(rig
            .push_secondary(BONE_PINNED, 1.0, 0.0, [0.0, -1.0, 0.0])
            .is_none());
        assert!(rig
            .push_secondary(99, 1.0, 0.0, [0.0, -1.0, 0.0])
            .is_none());
        assert!(rig
            .push_secondary(0, f32::NAN, 0.0, [0.0, -1.0, 0.0])
            .is_none());
        assert!(rig
            .push_secondary(0, 1.0, -0.1, [0.0, -1.0, 0.0])
            .is_none());
        assert!(rig
            .push_secondary(0, 1.0, 0.0, [f32::INFINITY, 0.0, 0.0])
            .is_none());
        assert!(rig
            .push_pinned(AvatarAnchorKind::Head, [f32::NAN, 0.0, 0.0])
            .is_none());
        assert_eq!(rig.bone_count(), 1);
    }

    #[test]
    fn non_finite_anchor_keeps_last_pin_pose() {
        let mut rig = AvatarRigBuffer::with_capacity(1);
        rig.push_pinned(AvatarAnchorKind::Head, [0.0, 0.0, 0.0])
            .unwrap();
        let mut world = AvatarAnchorWorld::default();
        assert!(world.set(AvatarAnchorKind::Head, [1.0, 2.0, 3.0]));
        step(&mut rig, &world, 1.0 / 60.0, 1, [0.0, 0.0, 0.0]);
        assert_eq!([rig.pos_x(0), rig.pos_y(0), rig.pos_z(0)], [1.0, 2.0, 3.0]);

        assert!(
            !world.set(AvatarAnchorKind::Head, [f32::NAN, 2.0, 3.0]),
            "set rejects non-finite (symmetric with push_pinned)"
        );
        assert_eq!(world.get(AvatarAnchorKind::Head), Some([1.0, 2.0, 3.0]));
        step(&mut rig, &world, 1.0 / 60.0, 1, [0.0, 0.0, 0.0]);
        assert_eq!(
            [rig.pos_x(0), rig.pos_y(0), rig.pos_z(0)],
            [1.0, 2.0, 3.0],
            "rejected non-finite set must leave pin at last good pose"
        );
    }

    #[test]
    fn pinned_extra_snaps_via_world() {
        let mut rig = AvatarRigBuffer::with_capacity(1);
        let idx = rig
            .push_pinned(AvatarAnchorKind::Extra(0), [0.0, 0.0, 0.0])
            .unwrap();
        assert_eq!(idx, 0);
        assert_eq!(
            AvatarAnchorKind::from_discriminant(rig.pin_discriminant(0)),
            Some(AvatarAnchorKind::Extra(0))
        );
        let mut world = AvatarAnchorWorld::default();
        assert!(world.set(AvatarAnchorKind::Extra(0), [4.0, 5.0, 6.0]));
        step(&mut rig, &world, 1.0 / 60.0, 1, [0.0, 0.0, 0.0]);
        assert_eq!([rig.pos_x(0), rig.pos_y(0), rig.pos_z(0)], [4.0, 5.0, 6.0]);
    }
}
