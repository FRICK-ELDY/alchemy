//! Host live edit ledger + Ready inbox (x00b3 / VCS working-tree tip).
//!
//! Contract (x00b3):
//! - Optimistic ledger mirrors mutate on live accept; **`tip` is NOT bumped** until FrameBridge
//!   queue **and** barrier succeed.
//! - Accept `EditCommit` only after that success (never rewind tip).
//! - Queue / barrier / inbox failure → `rollback_mirrors` + reject `EditCommit` (`APPLY`);
//!   tip untouched. Other clients may briefly see STALE on optimistic mirrors (retry OK).
//!
//! Non-guarantees: world overlay persistence; Save / Spatial VCS flush.

use std::collections::{HashMap, HashSet};
use std::sync::mpsc::{self, Receiver, Sender};
use std::sync::{Arc, Mutex};

use alchemy_protocol::wire_aead::AeadKind;
use alchemy_protocol::wire_crypto::{sign_edit_commit, SigningKey};
use alchemy_protocol::{
    edit_field_id, EditCommit, EditProposal, EditReparent, EditSetField, Message,
};
use zenoh::Wait;

use crate::room_aead::{self, SharedRoomAead};

/// Shared live working-tree tip (edit lane + physics).
#[derive(Debug, Default)]
pub struct LiveEditLedger {
    /// Monotonic EditCommit tip (advanced only after barrier success).
    pub tip: u64,
    /// child → parent entity id; missing or 0 = world root.
    pub parents: HashMap<u32, u32>,
    pub flags: HashMap<u32, u32>,
    pub colors: HashMap<u32, [f32; 4]>,
    pub blake3: HashMap<u32, String>,
    pub entities: HashSet<u32>,
}

pub type SharedLiveEdit = Arc<Mutex<LiveEditLedger>>;

pub fn new_shared_live_edit() -> SharedLiveEdit {
    Arc::new(Mutex::new(LiveEditLedger::default()))
}

impl LiveEditLedger {
    pub fn bump_tip(&mut self) -> u64 {
        self.tip = self.tip.saturating_add(1);
        self.tip
    }

    pub fn parent_of(&self, child: u32) -> u32 {
        self.parents.get(&child).copied().unwrap_or(0)
    }

    pub fn would_cycle(&self, child: u32, new_parent: u32) -> bool {
        if new_parent == 0 {
            return false;
        }
        if new_parent == child {
            return true;
        }
        let mut cur = new_parent;
        for _ in 0..=self.entities.len().saturating_add(1) {
            if cur == child {
                return true;
            }
            if cur == 0 {
                return false;
            }
            cur = self.parent_of(cur);
        }
        true
    }

    pub fn note_spawn(&mut self, entity_id: u32, flags: u32, color: [f32; 4], blake3: String) {
        self.entities.insert(entity_id);
        self.flags.insert(entity_id, flags);
        self.colors.insert(entity_id, color);
        if !blake3.is_empty() {
            self.blake3.insert(entity_id, blake3);
        }
        self.parents.entry(entity_id).or_insert(0);
    }

    pub fn note_despawn(&mut self, entity_id: u32) {
        self.entities.remove(&entity_id);
        self.parents.remove(&entity_id);
        self.flags.remove(&entity_id);
        self.colors.remove(&entity_id);
        self.blake3.remove(&entity_id);
        for p in self.parents.values_mut() {
            if *p == entity_id {
                *p = 0;
            }
        }
    }

    /// Restore mirrors after a live op that never completed barrier (tip unchanged).
    pub fn rollback_mirrors(&mut self, ready: &ReadyLiveEdit) {
        match ready {
            ReadyLiveEdit::Reparent {
                op, prev_parent, ..
            } => {
                self.parents.insert(op.child_entity_id, *prev_parent);
            }
            ReadyLiveEdit::SetField { op, prev_value, .. } => match op.field_id {
                edit_field_id::FLAGS => {
                    if let Some(f) = alchemy_protocol::decode_flags(prev_value) {
                        self.flags.insert(op.entity_id, f);
                    }
                }
                edit_field_id::COLOR_RGBA => {
                    if let Some(c) = alchemy_protocol::decode_color_rgba(prev_value) {
                        self.colors.insert(op.entity_id, c);
                    }
                }
                edit_field_id::BLAKE3 => {
                    if let Ok(hex) = std::str::from_utf8(prev_value) {
                        self.blake3.insert(op.entity_id, hex.to_ascii_lowercase());
                    }
                }
                _ => {}
            },
        }
    }
}

/// Accepted live op awaiting FrameBridge queue + barrier before tip/Commit.
#[derive(Clone, Debug)]
pub enum ReadyLiveEdit {
    Reparent {
        proposal: EditProposal,
        op: EditReparent,
        prev_parent: u32,
    },
    SetField {
        proposal: EditProposal,
        op: EditSetField,
        prev_value: Vec<u8>,
    },
}

impl ReadyLiveEdit {
    pub fn proposal(&self) -> &EditProposal {
        match self {
            Self::Reparent { proposal, .. } | Self::SetField { proposal, .. } => proposal,
        }
    }
}

pub struct LiveEditInbox {
    rx: Mutex<Receiver<ReadyLiveEdit>>,
}

impl LiveEditInbox {
    pub fn pair() -> (Sender<ReadyLiveEdit>, Self) {
        let (tx, rx) = mpsc::channel();
        (tx, Self { rx: Mutex::new(rx) })
    }

    pub fn try_recv(&self) -> Option<ReadyLiveEdit> {
        self.rx.lock().ok()?.try_recv().ok()
    }

    pub fn try_recv_n(&self, max: usize) -> Vec<ReadyLiveEdit> {
        let mut out = Vec::new();
        for _ in 0..max {
            match self.try_recv() {
                Some(r) => out.push(r),
                None => break,
            }
        }
        out
    }

    #[cfg(test)]
    pub fn empty_for_test() -> Self {
        let (_tx, rx) = mpsc::channel();
        Self { rx: Mutex::new(rx) }
    }
}

/// Field bytes currently stored for `field_id` (for hash compare).
pub fn current_field_value(
    ledger: &LiveEditLedger,
    entity_id: u32,
    field_id: u32,
) -> Option<Vec<u8>> {
    match field_id {
        edit_field_id::FLAGS => {
            let f = ledger.flags.get(&entity_id).copied().unwrap_or(0);
            Some(alchemy_protocol::encode_flags(f).to_vec())
        }
        edit_field_id::COLOR_RGBA => {
            let c = ledger
                .colors
                .get(&entity_id)
                .copied()
                .unwrap_or([1.0, 1.0, 1.0, 1.0]);
            Some(alchemy_protocol::encode_color_rgba(c).to_vec())
        }
        edit_field_id::BLAKE3 => {
            let hex = ledger.blake3.get(&entity_id)?.clone();
            Some(hex.into_bytes())
        }
        _ => None,
    }
}

/// Publishes accept `EditCommit` after barrier (host physics tick).
pub struct LiveCommitPublisher {
    session: zenoh::Session,
    edit_key: String,
    room_id: String,
    room_aead: SharedRoomAead,
    host_key: SigningKey,
}

impl LiveCommitPublisher {
    pub fn new(
        session: zenoh::Session,
        edit_key: String,
        room_id: String,
        room_aead: SharedRoomAead,
        host_key: SigningKey,
    ) -> Self {
        Self {
            session,
            edit_key,
            room_id,
            room_aead,
            host_key,
        }
    }

    pub fn publish_accept(&self, proposal: &EditProposal, scene_rev: u64) {
        self.publish(
            proposal,
            scene_rev,
            true,
            alchemy_protocol::edit_reject::NONE,
        );
    }

    /// Queue / barrier / inbox failure NACK (tip unchanged).
    pub fn publish_reject(&self, proposal: &EditProposal, reason: u32, scene_rev: u64) {
        self.publish(proposal, scene_rev, false, reason);
    }

    fn publish(&self, proposal: &EditProposal, scene_rev: u64, accepted: bool, reject_reason: u32) {
        let mut commit = EditCommit {
            scene_rev,
            op_id: proposal.op_id,
            author_session_pub: proposal.session_pub.clone(),
            accepted,
            reject_reason,
            ops: proposal.ops.clone(),
            host_sig: vec![],
        };
        sign_edit_commit(&self.host_key, &mut commit);
        let plain = commit.encode_to_vec();
        match room_aead::seal_shared(&self.room_aead, &self.room_id, AeadKind::EditCommit, &plain) {
            Ok(env) => {
                if let Err(e) = self.session.put(&self.edit_key, env).wait() {
                    log::warn!("live EditCommit put failed: {e}");
                }
            }
            Err(e) => log::warn!("live EditCommit seal failed: {e}"),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use alchemy_protocol::wire_crypto::{
        generate_signing_key, public_key_bytes, sign_edit_proposal,
    };

    fn proposal() -> EditProposal {
        let key = generate_signing_key();
        let mut p = EditProposal {
            session_pub: public_key_bytes(&key),
            control_seq: 1,
            base_scene_rev: 0,
            op_id: 1,
            ops: vec![],
            sig: vec![],
            room_id: "local".into(),
        };
        sign_edit_proposal(&key, &mut p);
        p
    }

    #[test]
    fn rollback_mirrors_does_not_touch_tip() {
        let mut led = LiveEditLedger::default();
        led.entities.insert(10);
        led.parents.insert(10, 1);
        led.tip = 7;
        let ready = ReadyLiveEdit::Reparent {
            proposal: proposal(),
            op: EditReparent {
                child_entity_id: 10,
                expected_parent_entity_id: 1,
                new_parent_entity_id: 2,
            },
            prev_parent: 1,
        };
        led.parents.insert(10, 2);
        led.rollback_mirrors(&ready);
        assert_eq!(led.tip, 7);
        assert_eq!(led.parent_of(10), 1);
    }
}
