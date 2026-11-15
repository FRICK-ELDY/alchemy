//! Client Intent session key (x0023 / x003e / x0081 / x00b2).
//!
//! Guarantees: bind Intent carries Room Token until enough successful puts; steady
//! Intents are Ed25519-signed thereafter. Room Token must be issued for this
//! session_pub (`cnf.spk`, x003e). Edit-lane Spawn/Despawn/Reparent/SetField/Save (and legacy
//! inventory JSON helpers) reuse the same key with a separate `control_seq` — no Room Token on Zenoh.
//! Non-guarantees: host acceptance ack (Zenoh put ≠ verify); key persistence across restart.

use alchemy_protocol::control_plane::{self, ControlPose};
use alchemy_protocol::edit_op::Op;
use alchemy_protocol::wire_crypto::{
    generate_signing_key, public_key_bytes, sign_edit_proposal, sign_input, SigningKey,
};
use alchemy_protocol::{EditDespawn, EditOp, EditProposal, EditSave, EditSpawn, InputEvent};
use base64::Engine;

/// Successful Zenoh puts that must still include `room_token` after join / refresh.
/// Covers put failure retries and brief host reject windows without an ack channel.
const BIND_PUTS_REQUIRED: u32 = 3;

/// Non-auth fields for a signed Intent (keeps `signed_intent` under clippy arg limits).
#[derive(Clone, Copy, Default)]
pub struct IntentBody {
    pub client_time_ms: u64,
    pub move_x: f32,
    pub move_y: f32,
    pub move_z: f32,
    pub buttons: u32,
    pub grab_entity_id: u32,
    pub aim_x: f32,
    pub aim_y: f32,
    pub aim_z: f32,
    pub leave: bool,
    /// World-space release linear velocity (m/s) when releasing Grab (x00af).
    pub release_vx: f32,
    pub release_vy: f32,
    pub release_vz: f32,
    /// World-space release angular velocity (rad/s) when releasing Grab (x00af).
    pub release_wx: f32,
    pub release_wy: f32,
    pub release_wz: f32,
}

/// Legacy inventory JSON control signature payload (x0081).
///
/// Not used by x00b2 client publish (see [`IntentSession::sign_edit_spawn`]); retained for
/// host dual-subscribe probes / unit tests of `control_plane` signing.
#[derive(Clone)]
#[allow(dead_code)]
pub struct ControlSig {
    pub session_pub_b64: String,
    pub control_seq: u64,
    pub control_sig_b64: String,
}

#[derive(Clone)]
pub struct IntentSession {
    key: SigningKey,
    seq: u64,
    /// Inventory control-plane sequence (x0081); independent of Intent `intent_seq`.
    control_seq: u64,
    /// Remaining successful puts that must carry `room_token`.
    bind_puts_remaining: u32,
}

impl IntentSession {
    pub fn new() -> Self {
        Self {
            key: generate_signing_key(),
            seq: 0,
            control_seq: 0,
            bind_puts_remaining: BIND_PUTS_REQUIRED,
        }
    }

    pub fn mark_bind_pending(&mut self) {
        self.bind_puts_remaining = BIND_PUTS_REQUIRED;
    }

    pub fn bind_pending(&self) -> bool {
        self.bind_puts_remaining > 0
    }

    /// Raw 32-byte Ed25519 public key (Intent `session_pub`).
    pub fn session_pub(&self) -> [u8; 32] {
        self.key.verifying_key().to_bytes()
    }

    /// base64url (no pad) for `POST …/tokens` `session_pub` / JWT `cnf.spk` (x003e).
    pub fn session_pub_b64(&self) -> String {
        base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(self.session_pub())
    }

    /// Call only after Zenoh `put` succeeded for an Intent that included `room_token`.
    pub fn note_bind_put_ok(&mut self) {
        self.bind_puts_remaining = self.bind_puts_remaining.saturating_sub(1);
    }

    /// Build a signed Intent. Does **not** clear bind state (caller does after put OK).
    pub fn signed_intent(&mut self, room_token: Option<&str>, body: IntentBody) -> InputEvent {
        self.seq = self.seq.saturating_add(1);
        let include_token = self.bind_puts_remaining > 0;
        let token = if include_token {
            room_token.unwrap_or("").to_string()
        } else {
            String::new()
        };
        let mut event = InputEvent {
            room_token: token,
            client_time_ms: body.client_time_ms,
            move_x: body.move_x,
            move_y: body.move_y,
            move_z: body.move_z,
            buttons: body.buttons,
            grab_entity_id: body.grab_entity_id,
            aim_x: body.aim_x,
            aim_y: body.aim_y,
            aim_z: body.aim_z,
            leave: body.leave,
            session_pub: self.session_pub().to_vec(),
            intent_seq: self.seq,
            intent_sig: vec![],
            release_vx: body.release_vx,
            release_vy: body.release_vy,
            release_vz: body.release_vz,
            release_wx: body.release_wx,
            release_wy: body.release_wy,
            release_wz: body.release_wz,
        };
        sign_input(&self.key, &mut event);
        event
    }

    /// Sign legacy inventory JSON spawn (`control_plane` bytes; advances `control_seq`).
    ///
    /// **Dead client publish path (x00b2):** kept for unit tests / dual-subscribe host
    /// compatibility probes. Production UI must use [`Self::sign_edit_spawn`].
    #[allow(dead_code)]
    pub fn sign_control_spawn(
        &mut self,
        room_id: &str,
        blake3_hex: &str,
        pose: ControlPose,
    ) -> ControlSig {
        self.control_seq = self.control_seq.saturating_add(1);
        let blake3 = blake3_hex.to_ascii_lowercase();
        let sig = control_plane::sign_spawn(&self.key, room_id, self.control_seq, &blake3, pose);
        ControlSig {
            session_pub_b64: self.session_pub_b64(),
            control_seq: self.control_seq,
            control_sig_b64: base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(sig),
        }
    }

    /// Sign legacy inventory JSON despawn. Same dead-publish policy as [`Self::sign_control_spawn`].
    #[allow(dead_code)]
    pub fn sign_control_despawn(&mut self, room_id: &str, entity_id: u32) -> ControlSig {
        self.control_seq = self.control_seq.saturating_add(1);
        let sig = control_plane::sign_despawn(&self.key, room_id, self.control_seq, entity_id);
        ControlSig {
            session_pub_b64: self.session_pub_b64(),
            control_seq: self.control_seq,
            control_sig_b64: base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(sig),
        }
    }

    /// Build a signed `EditProposal` Spawn (x00b2; advances `control_seq`).
    ///
    /// `base_scene_rev` must be the last observed host live `EditCommit.scene_rev` (or 0).
    pub fn sign_edit_spawn(
        &mut self,
        room_id: &str,
        base_scene_rev: u64,
        blake3_hex: &str,
        pose: ControlPose,
        spawn_ticket: &str,
    ) -> EditProposal {
        self.control_seq = self.control_seq.saturating_add(1);
        let blake3 = blake3_hex.to_ascii_lowercase();
        let mut proposal = EditProposal {
            session_pub: public_key_bytes(&self.key),
            control_seq: self.control_seq,
            base_scene_rev,
            op_id: self.control_seq,
            ops: vec![EditOp {
                op: Some(Op::Spawn(EditSpawn {
                    blake3,
                    x: pose[0],
                    y: pose[1],
                    z: pose[2],
                    qx: pose[3],
                    qy: pose[4],
                    qz: pose[5],
                    qw: pose[6],
                    sx: pose[7],
                    sy: pose[8],
                    sz: pose[9],
                    spawn_ticket: spawn_ticket.to_string(),
                })),
            }],
            sig: vec![],
            room_id: room_id.to_string(),
        };
        sign_edit_proposal(&self.key, &mut proposal);
        proposal
    }

    /// Build a signed `EditProposal` Despawn (x00b2; advances `control_seq`).
    pub fn sign_edit_despawn(
        &mut self,
        room_id: &str,
        base_scene_rev: u64,
        entity_id: u32,
    ) -> EditProposal {
        self.control_seq = self.control_seq.saturating_add(1);
        let mut proposal = EditProposal {
            session_pub: public_key_bytes(&self.key),
            control_seq: self.control_seq,
            base_scene_rev,
            op_id: self.control_seq,
            ops: vec![EditOp {
                op: Some(Op::Despawn(EditDespawn { entity_id })),
            }],
            sig: vec![],
            room_id: room_id.to_string(),
        };
        sign_edit_proposal(&self.key, &mut proposal);
        proposal
    }

    /// Build a signed `EditProposal` Reparent (x00b3; advances `control_seq`).
    pub fn sign_edit_reparent(
        &mut self,
        room_id: &str,
        base_scene_rev: u64,
        child_entity_id: u32,
        expected_parent_entity_id: u32,
        new_parent_entity_id: u32,
    ) -> EditProposal {
        self.control_seq = self.control_seq.saturating_add(1);
        let mut proposal = EditProposal {
            session_pub: public_key_bytes(&self.key),
            control_seq: self.control_seq,
            base_scene_rev,
            op_id: self.control_seq,
            ops: vec![EditOp {
                op: Some(Op::Reparent(alchemy_protocol::EditReparent {
                    child_entity_id,
                    expected_parent_entity_id,
                    new_parent_entity_id,
                })),
            }],
            sig: vec![],
            room_id: room_id.to_string(),
        };
        sign_edit_proposal(&self.key, &mut proposal);
        proposal
    }

    /// Build a signed `EditProposal` SetField (x00b3; advances `control_seq`).
    pub fn sign_edit_set_field(
        &mut self,
        room_id: &str,
        base_scene_rev: u64,
        entity_id: u32,
        field_id: u32,
        value: Vec<u8>,
        expected_value_hash: Vec<u8>,
    ) -> EditProposal {
        self.control_seq = self.control_seq.saturating_add(1);
        let mut proposal = EditProposal {
            session_pub: public_key_bytes(&self.key),
            control_seq: self.control_seq,
            base_scene_rev,
            op_id: self.control_seq,
            ops: vec![EditOp {
                op: Some(Op::SetField(alchemy_protocol::EditSetField {
                    entity_id,
                    field_id,
                    value,
                    expected_value_hash,
                })),
            }],
            sig: vec![],
            room_id: room_id.to_string(),
        };
        sign_edit_proposal(&self.key, &mut proposal);
        proposal
    }

    /// Build a signed `EditProposal` Save (x00b7; advances `control_seq`).
    pub fn sign_edit_save(&mut self, room_id: &str, base_scene_rev: u64) -> EditProposal {
        self.control_seq = self.control_seq.saturating_add(1);
        let mut proposal = EditProposal {
            session_pub: public_key_bytes(&self.key),
            control_seq: self.control_seq,
            base_scene_rev,
            op_id: self.control_seq,
            ops: vec![EditOp {
                op: Some(Op::Save(EditSave {})),
            }],
            sig: vec![],
            room_id: room_id.to_string(),
        };
        sign_edit_proposal(&self.key, &mut proposal);
        proposal
    }
}

impl Default for IntentSession {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bind_clears_only_after_note_put_ok() {
        let mut s = IntentSession::new();
        assert!(s.bind_pending());
        let ev = s.signed_intent(Some("jwt"), IntentBody::default());
        assert!(!ev.room_token.is_empty());
        assert!(s.bind_pending());
        s.note_bind_put_ok();
        assert!(s.bind_pending());
        s.note_bind_put_ok();
        s.note_bind_put_ok();
        assert!(!s.bind_pending());
        let steady = s.signed_intent(Some("jwt"), IntentBody::default());
        assert!(steady.room_token.is_empty());
    }
}
