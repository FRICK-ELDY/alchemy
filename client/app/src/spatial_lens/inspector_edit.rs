//! Inspector POS scrub → `…/edit` SetField Commit (x00f2 / x00f4).
//!
//! Guarantees: build signed SetField(s) from Epoch expected hash; one Commit per release;
//! dirty axes only in one proposal (1..=3 POS ops · x00f4).
//! Non-guarantees: SoftLease chrome (→ extract / x00f3); XR scrub; Save pose fold;
//! Zenoh volatile scrub stream; Grab release pose Commit.

#![forbid(unsafe_code)]

use alchemy_protocol::hash_f64;
use anyhow::{anyhow, Result};

use crate::inventory::publish_edit_proposal;
use crate::scene::SharedState;
use crate::spatial_lens::extract::{pos_field_id_for_axis, InspectorScrub};

#[cfg(test)]
use crate::intent_session::IntentSession;
#[cfg(test)]
use alchemy_protocol::EditProposal;

/// One dirty axis ready for Commit (wire field + draft + expected hash).
#[derive(Clone, Debug)]
pub struct PosAxisCommit {
    pub field_id: u32,
    pub value: f64,
    pub expected_hash: [u8; 32],
}

/// Collect dirty axes from a finished scrub (skips non-finite drafts).
pub fn dirty_pos_commits(scrub: &InspectorScrub) -> Vec<PosAxisCommit> {
    let mut out = Vec::with_capacity(3);
    for axis in 0..3 {
        if !scrub.dirty[axis] {
            continue;
        }
        let Some(field_id) = pos_field_id_for_axis(axis) else {
            continue;
        };
        let value = scrub.draft_xyz[axis];
        if !value.is_finite() {
            continue;
        }
        out.push(PosAxisCommit {
            field_id,
            value,
            expected_hash: scrub.expected_hash[axis],
        });
    }
    out
}

/// Sign + put SetField(s) for a finished Inspector scrub (one proposal · x00f4).
pub fn publish_inspector_pos_commit(
    shared: &SharedState,
    zenoh: &zenoh::Session,
    scrub: &InspectorScrub,
) -> Result<()> {
    let axes = dirty_pos_commits(scrub);
    if axes.is_empty() {
        return Ok(());
    }
    let Some(wire_cap) = shared.wire_cap() else {
        return Err(anyhow!("no wire_cap"));
    };
    let Some((room_key, room_key_epoch)) = shared.live_room_aead() else {
        return Err(anyhow!("room_key missing"));
    };
    let room_id = shared.get_room_id();
    let proposal = {
        let Ok(mut gate) = shared.intent_session.lock() else {
            return Err(anyhow!("intent_session lock failed"));
        };
        let Some(intent) = gate.as_mut() else {
            return Err(anyhow!("Intent session required"));
        };
        let base_scene_rev = shared
            .last_edit_scene_rev
            .lock()
            .ok()
            .and_then(|g| *g)
            .unwrap_or(0);
        intent.sign_edit_set_fields_pos(room_id.as_ref(), base_scene_rev, scrub.entity_id, &axes)
    };
    publish_edit_proposal(
        zenoh,
        room_id.as_ref(),
        wire_cap.as_ref(),
        room_key.as_ref(),
        room_key_epoch,
        &proposal,
    )?;
    log::info!(
        "inspector SetField POS entity={} axes={} op_id={}",
        scrub.entity_id,
        axes.len(),
        proposal.op_id
    );
    Ok(())
}

/// Hash helper for scrub begin (mirrors host Epoch check).
#[inline]
pub fn expected_hash_for_axis(value: f64) -> [u8; 32] {
    hash_f64(value)
}

/// Test-only: build proposal without Zenoh (hash / field wiring).
///
/// Mirrors [`publish_inspector_pos_commit`]: empty dirty → `None` (no sign / no ops).
#[cfg(test)]
pub fn sign_inspector_pos_for_test(
    intent: &mut IntentSession,
    room_id: &str,
    scrub: &InspectorScrub,
) -> Option<EditProposal> {
    let axes = dirty_pos_commits(scrub);
    if axes.is_empty() {
        return None;
    }
    Some(intent.sign_edit_set_fields_pos(room_id, 0, scrub.entity_id, &axes))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::spatial_lens::extract::INSPECT_FIELD_POS_X;
    use alchemy_protocol::encode_f64;

    fn scrub_xy_dirty() -> InspectorScrub {
        InspectorScrub {
            field_binding: INSPECT_FIELD_POS_X,
            row: 1,
            entity_id: 10,
            start_uv: [0.0, 0.0],
            last_uv: [0.0, 0.0],
            base_xyz: [1.0, 2.0, 3.0],
            draft_xyz: [2.5, 4.0, 3.0],
            dirty: [true, true, false],
            expected_hash: [hash_f64(1.0), hash_f64(2.0), hash_f64(3.0)],
        }
    }

    #[test]
    fn expected_hash_matches_encode() {
        let v = 1.5_f64;
        assert_eq!(expected_hash_for_axis(v), hash_f64(v));
        assert_eq!(
            expected_hash_for_axis(v),
            alchemy_protocol::field_value_hash(&encode_f64(v))
        );
    }

    #[test]
    fn dirty_pos_commits_skips_clean_axes() {
        let scrub = scrub_xy_dirty();
        let axes = dirty_pos_commits(&scrub);
        assert_eq!(axes.len(), 2);
        assert_eq!(axes[0].field_id, alchemy_protocol::edit_field_id::POS_X);
        assert_eq!(axes[1].field_id, alchemy_protocol::edit_field_id::POS_Y);
        assert_eq!(axes[0].value, 2.5);
        assert_eq!(axes[1].value, 4.0);
    }

    #[test]
    fn sign_set_fields_one_proposal_multi_ops() {
        let mut intent = IntentSession::new();
        let scrub = scrub_xy_dirty();
        let p = sign_inspector_pos_for_test(&mut intent, "room", &scrub).expect("dirty → proposal");
        assert_eq!(p.ops.len(), 2);
        assert_eq!(p.control_seq, 1);
        assert_eq!(p.op_id, 1);
        match p.ops[0].op.as_ref().unwrap() {
            alchemy_protocol::edit_op::Op::SetField(f) => {
                assert_eq!(f.entity_id, 10);
                assert_eq!(f.field_id, alchemy_protocol::edit_field_id::POS_X);
                assert_eq!(f.value, encode_f64(2.5).to_vec());
                assert_eq!(f.expected_value_hash, hash_f64(1.0).to_vec());
            }
            _ => panic!("expected SetField"),
        }
        match p.ops[1].op.as_ref().unwrap() {
            alchemy_protocol::edit_op::Op::SetField(f) => {
                assert_eq!(f.field_id, alchemy_protocol::edit_field_id::POS_Y);
                assert_eq!(f.value, encode_f64(4.0).to_vec());
            }
            _ => panic!("expected SetField"),
        }
        // Second call advances seq once more (proposal-unit).
        let p2 =
            sign_inspector_pos_for_test(&mut intent, "room", &scrub).expect("dirty → proposal");
        assert_eq!(p2.control_seq, 2);
        assert_eq!(p2.ops.len(), 2);
    }

    #[test]
    fn clean_scrub_yields_no_ops_contract() {
        // Mid-drag / no-delta release: dirty all false → 0 edit ops (no …/edit).
        let scrub = InspectorScrub {
            field_binding: INSPECT_FIELD_POS_X,
            row: 1,
            entity_id: 10,
            start_uv: [0.0, 0.0],
            last_uv: [0.0, 0.0],
            base_xyz: [1.0, 2.0, 3.0],
            draft_xyz: [1.0, 2.0, 3.0],
            dirty: [false; 3],
            expected_hash: [hash_f64(1.0), hash_f64(2.0), hash_f64(3.0)],
        };
        assert!(dirty_pos_commits(&scrub).is_empty());
        let mut intent = IntentSession::new();
        assert!(
            sign_inspector_pos_for_test(&mut intent, "room", &scrub).is_none(),
            "test helper matches publish early-return"
        );
    }
}
