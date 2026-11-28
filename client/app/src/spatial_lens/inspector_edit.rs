//! Inspector POS scrub → `…/edit` SetField Commit (x00f2).
//!
//! Guarantees: build signed SetField from Epoch expected hash; one Commit per release.
//! Non-guarantees: SoftLease UI; XR scrub; Save pose fold.

#![forbid(unsafe_code)]

use alchemy_protocol::{encode_f64, hash_f64};
use anyhow::{anyhow, Result};

use crate::inventory::publish_edit_proposal;
use crate::scene::SharedState;
use crate::spatial_lens::extract::InspectorScrub;

#[cfg(test)]
use crate::intent_session::IntentSession;
#[cfg(test)]
use alchemy_protocol::EditProposal;

/// Sign + put SetField for a finished Inspector scrub.
pub fn publish_inspector_pos_commit(
    shared: &SharedState,
    zenoh: &zenoh::Session,
    scrub: &InspectorScrub,
    field_id: u32,
) -> Result<()> {
    let Some(wire_cap) = shared.wire_cap() else {
        return Err(anyhow!("no wire_cap"));
    };
    let Some((room_key, room_key_epoch)) = shared.live_room_aead() else {
        return Err(anyhow!("room_key missing"));
    };
    let room_id = shared.get_room_id();
    let value = encode_f64(scrub.draft_value).to_vec();
    let expected = scrub.expected_hash.to_vec();
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
        intent.sign_edit_set_field(
            room_id.as_ref(),
            base_scene_rev,
            scrub.entity_id,
            field_id,
            value,
            expected,
        )
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
        "inspector SetField POS entity={} field={} value={:.4} op_id={}",
        scrub.entity_id,
        field_id,
        scrub.draft_value,
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
#[cfg(test)]
pub fn sign_inspector_pos_for_test(
    intent: &mut IntentSession,
    room_id: &str,
    scrub: &InspectorScrub,
    field_id: u32,
) -> EditProposal {
    intent.sign_edit_set_field(
        room_id,
        0,
        scrub.entity_id,
        field_id,
        encode_f64(scrub.draft_value).to_vec(),
        scrub.expected_hash.to_vec(),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::spatial_lens::extract::INSPECT_FIELD_POS_X;

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
    fn sign_set_field_carries_pos_x() {
        let mut intent = IntentSession::new();
        let scrub = InspectorScrub {
            field_binding: INSPECT_FIELD_POS_X,
            row: 1,
            entity_id: 10,
            start_uv_x: 0.0,
            last_uv: [0.0, 0.0],
            base_value: 1.0,
            draft_value: 2.5,
            expected_hash: hash_f64(1.0),
        };
        let p = sign_inspector_pos_for_test(
            &mut intent,
            "room",
            &scrub,
            alchemy_protocol::edit_field_id::POS_X,
        );
        assert_eq!(p.ops.len(), 1);
        match p.ops[0].op.as_ref().unwrap() {
            alchemy_protocol::edit_op::Op::SetField(f) => {
                assert_eq!(f.entity_id, 10);
                assert_eq!(f.field_id, alchemy_protocol::edit_field_id::POS_X);
                assert_eq!(f.value, encode_f64(2.5).to_vec());
                assert_eq!(f.expected_value_hash, hash_f64(1.0).to_vec());
            }
            _ => panic!("expected SetField"),
        }
    }
}
