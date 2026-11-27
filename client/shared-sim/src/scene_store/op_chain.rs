//! Host-sequenced op chain hash (x00e5 枠).
//!
//! Guarantees: deterministic [`digest_crdt_op`] + [`fold_op_chain_hash`];
//! domain-separated canonical bytes ([`OP_CHAIN_DOMAIN`]);
//! [`SequencedOp`] data shape matching roadmap §6 (Phase 1b `EntityId` keys).
//! Non-guarantees: wire / `EditCommit` / PROTOCOL_PIN carriage of
//! `op_chain_hash`; host_loop tip-path auto-fold; World.Verifier Spot-Check
//! stats (Proprietary — OSS Behaviour injection only).
//!
//! Hash primitive: [`alchemy_protocol::blake3_hash_32`] (general BLAKE3), **not**
//! SetField [`alchemy_protocol::field_value_hash`] semantics. Same blake3 crate
//! via protocol; do not treat SetField lock-hash format changes as op_chain
//! schema changes (domain prefix owns the op_chain schema).

use alchemy_protocol::blake3_hash_32;

use super::{CrdtOp, OpTimestamp, PropValue};

/// Domain separation prefix for op_chain digests / folds (v1).
///
/// Bump / rename when tag numbering or canonical layout changes incompatibly
/// (before PROTOCOL_PIN carriage).
pub const OP_CHAIN_DOMAIN: &[u8] = b"alchemy.op_chain.v1\0";

/// Confirmed Scene Store op with host sequencing + chain tip (roadmap §6).
///
/// Wire today remains `EditCommit` / `scene_rev`; this type is the in-process
/// skeleton for Late Join / Verifier inputs.
#[derive(Clone, Debug, PartialEq)]
pub struct SequencedOp {
    /// ↔ `host_generation`.
    pub host_epoch: u64,
    /// ↔ `scene_rev` / `LiveEditLedger.tip`.
    pub room_seq: u64,
    /// Chain tip **after** folding this op.
    pub op_chain_hash: [u8; 32],
    pub op: CrdtOp,
}

/// BLAKE3 digest of a [`CrdtOp`] (deterministic canonical bytes).
///
/// Allocates a scratch `Vec` — fine for 枠 / tests. Prefer
/// [`digest_crdt_op_into`] on a hot tip path to reuse the buffer.
pub fn digest_crdt_op(op: &CrdtOp) -> [u8; 32] {
    let mut buf = Vec::with_capacity(OP_CHAIN_DOMAIN.len() + 64);
    digest_crdt_op_into(op, &mut buf)
}

/// Like [`digest_crdt_op`] but reuses `buf` (cleared then filled).
pub fn digest_crdt_op_into(op: &CrdtOp, buf: &mut Vec<u8>) -> [u8; 32] {
    buf.clear();
    buf.extend_from_slice(OP_CHAIN_DOMAIN);
    append_crdt_op(buf, op);
    blake3_hash_32(buf)
}

/// Fixed fold payload size: domain + prev(32) + epoch(8) + seq(8) + digest(32).
const FOLD_BUF_LEN: usize = OP_CHAIN_DOMAIN.len() + 32 + 8 + 8 + 32;

/// Fold one confirmed op into the chain tip.
///
/// `prev` is the hash **before** this op; result is the tip **after**.
/// Bytes: `OP_CHAIN_DOMAIN ‖ prev ‖ host_epoch_le ‖ room_seq_le ‖ op_digest`
/// (stack buffer — no heap alloc).
pub fn fold_op_chain_hash(
    prev: &[u8; 32],
    host_epoch: u64,
    room_seq: u64,
    op_digest: &[u8; 32],
) -> [u8; 32] {
    let mut buf = [0u8; FOLD_BUF_LEN];
    let mut i = 0;
    buf[i..i + OP_CHAIN_DOMAIN.len()].copy_from_slice(OP_CHAIN_DOMAIN);
    i += OP_CHAIN_DOMAIN.len();
    buf[i..i + 32].copy_from_slice(prev);
    i += 32;
    buf[i..i + 8].copy_from_slice(&host_epoch.to_le_bytes());
    i += 8;
    buf[i..i + 8].copy_from_slice(&room_seq.to_le_bytes());
    i += 8;
    buf[i..i + 32].copy_from_slice(op_digest);
    blake3_hash_32(&buf)
}

/// Convenience: digest + fold in one step (allocates digest scratch only).
pub fn fold_op_chain(prev: &[u8; 32], host_epoch: u64, room_seq: u64, op: &CrdtOp) -> [u8; 32] {
    fold_op_chain_hash(prev, host_epoch, room_seq, &digest_crdt_op(op))
}

/// Like [`fold_op_chain`] with a reusable **digest** scratch buffer.
///
/// Fold payload itself is always a stack array ([`fold_op_chain_hash`]).
pub fn fold_op_chain_into(
    prev: &[u8; 32],
    host_epoch: u64,
    room_seq: u64,
    op: &CrdtOp,
    digest_buf: &mut Vec<u8>,
) -> [u8; 32] {
    let digest = digest_crdt_op_into(op, digest_buf);
    fold_op_chain_hash(prev, host_epoch, room_seq, &digest)
}

fn append_u8(buf: &mut Vec<u8>, v: u8) {
    buf.push(v);
}

fn append_u32(buf: &mut Vec<u8>, v: u32) {
    buf.extend_from_slice(&v.to_le_bytes());
}

fn append_u64(buf: &mut Vec<u8>, v: u64) {
    buf.extend_from_slice(&v.to_le_bytes());
}

fn append_f32(buf: &mut Vec<u8>, v: f32) {
    buf.extend_from_slice(&v.to_bits().to_le_bytes());
}

fn append_ts(buf: &mut Vec<u8>, ts: OpTimestamp) {
    append_u64(buf, ts.lamport);
    append_u64(buf, ts.actor_id);
}

fn append_opt_entity(buf: &mut Vec<u8>, id: Option<u32>) {
    match id {
        None => append_u8(buf, 0),
        Some(e) => {
            append_u8(buf, 1);
            append_u32(buf, e);
        }
    }
}

fn append_prop_value(buf: &mut Vec<u8>, value: &PropValue) {
    match value {
        PropValue::Bool(v) => {
            append_u8(buf, 1);
            append_u8(buf, u8::from(*v));
        }
        PropValue::U32(v) => {
            append_u8(buf, 2);
            append_u32(buf, *v);
        }
        PropValue::F32(v) => {
            append_u8(buf, 3);
            append_f32(buf, *v);
        }
        PropValue::F32x3(v) => {
            append_u8(buf, 4);
            for x in v {
                append_f32(buf, *x);
            }
        }
        PropValue::F32x4(v) => {
            append_u8(buf, 5);
            for x in v {
                append_f32(buf, *x);
            }
        }
        PropValue::Bytes(v) => {
            append_u8(buf, 6);
            let len = u32::try_from(v.len())
                .expect("PropValue::Bytes length exceeds u32::MAX (op_chain digest contract)");
            append_u32(buf, len);
            buf.extend_from_slice(v);
        }
        PropValue::Blake3(v) => {
            append_u8(buf, 7);
            buf.extend_from_slice(v);
        }
    }
}

fn append_crdt_op(buf: &mut Vec<u8>, op: &CrdtOp) {
    match op {
        CrdtOp::SpawnNode { ts, node, parent } => {
            append_u8(buf, 1);
            append_ts(buf, *ts);
            append_u32(buf, *node);
            append_opt_entity(buf, *parent);
        }
        CrdtOp::MoveNode {
            ts,
            node,
            new_parent,
        } => {
            append_u8(buf, 2);
            append_ts(buf, *ts);
            append_u32(buf, *node);
            append_opt_entity(buf, *new_parent);
        }
        CrdtOp::SetProperty {
            ts,
            node,
            prop_key,
            value,
        } => {
            append_u8(buf, 3);
            append_ts(buf, *ts);
            append_u32(buf, *node);
            append_u32(buf, *prop_key);
            append_prop_value(buf, value);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::scene_store::OpTimestamp;

    fn ts() -> OpTimestamp {
        OpTimestamp {
            lamport: 1,
            actor_id: 7,
        }
    }

    fn spawn(node: u32, parent: Option<u32>) -> CrdtOp {
        CrdtOp::SpawnNode {
            ts: ts(),
            node,
            parent,
        }
    }

    fn set_prop(node: u32, prop_key: u32, value: PropValue) -> CrdtOp {
        CrdtOp::SetProperty {
            ts: ts(),
            node,
            prop_key,
            value,
        }
    }

    fn hex32(h: &[u8; 32]) -> String {
        h.iter().map(|b| format!("{b:02x}")).collect()
    }

    #[test]
    fn digest_is_deterministic() {
        let op = spawn(10, None);
        assert_eq!(digest_crdt_op(&op), digest_crdt_op(&op));
    }

    #[test]
    fn digest_into_matches_digest() {
        let op = spawn(10, Some(1));
        let mut buf = Vec::new();
        assert_eq!(digest_crdt_op_into(&op, &mut buf), digest_crdt_op(&op));
        // Reuse buffer.
        let again = digest_crdt_op_into(&op, &mut buf);
        assert_eq!(again, digest_crdt_op(&op));
    }

    #[test]
    fn different_ops_diverge() {
        let a = spawn(10, None);
        let b = spawn(11, None);
        assert_ne!(digest_crdt_op(&a), digest_crdt_op(&b));
    }

    #[test]
    fn fold_chain_from_zero_is_deterministic() {
        let zero = [0u8; 32];
        let op1 = spawn(1, None);
        let op2 = CrdtOp::MoveNode {
            ts: OpTimestamp {
                lamport: 2,
                actor_id: 7,
            },
            node: 1,
            new_parent: Some(2),
        };
        let h1 = fold_op_chain(&zero, 5, 1, &op1);
        let h2 = fold_op_chain(&h1, 5, 2, &op2);
        let h1b = fold_op_chain(&zero, 5, 1, &op1);
        let h2b = fold_op_chain(&h1b, 5, 2, &op2);
        assert_eq!(h1, h1b);
        assert_eq!(h2, h2b);
        assert_ne!(h1, zero);
        assert_ne!(h2, h1);
    }

    #[test]
    fn different_room_seq_diverges() {
        let zero = [0u8; 32];
        let op = spawn(1, None);
        let a = fold_op_chain(&zero, 1, 1, &op);
        let b = fold_op_chain(&zero, 1, 2, &op);
        assert_ne!(a, b);
    }

    #[test]
    fn sequenced_op_holds_folded_tip() {
        let zero = [0u8; 32];
        let op = spawn(3, Some(1));
        let tip = fold_op_chain(&zero, 9, 4, &op);
        let seq = SequencedOp {
            host_epoch: 9,
            room_seq: 4,
            op_chain_hash: tip,
            op,
        };
        assert_eq!(seq.op_chain_hash, tip);
        assert_eq!(seq.room_seq, 4);
    }

    #[test]
    fn set_property_all_prop_value_variants_deterministic() {
        let variants = [
            PropValue::Bool(true),
            PropValue::U32(0xDEAD_BEEF),
            PropValue::F32(1.5),
            PropValue::F32(f32::from_bits(0x7FC0_0000)), // quiet NaN bit pattern
            PropValue::F32x3([1.0, -2.0, 3.5]),
            PropValue::F32x4([0.0, 1.0, 2.0, 3.0]),
            PropValue::Bytes(vec![]),
            PropValue::Bytes(vec![0, 1, 2, 255]),
            PropValue::Bytes(vec![7; 64]),
            PropValue::Blake3([0xAB; 32]),
        ];
        for (i, value) in variants.into_iter().enumerate() {
            let op = set_prop(9, i as u32, value);
            let a = digest_crdt_op(&op);
            let b = digest_crdt_op(&op);
            assert_eq!(a, b, "variant {i}");
        }
        // Distinct payloads diverge.
        let a = digest_crdt_op(&set_prop(1, 0, PropValue::Bytes(vec![1])));
        let b = digest_crdt_op(&set_prop(1, 0, PropValue::Bytes(vec![1, 0])));
        assert_ne!(a, b);
        let nan_a = digest_crdt_op(&set_prop(1, 0, PropValue::F32(f32::from_bits(0x7FC0_0000))));
        let nan_b = digest_crdt_op(&set_prop(1, 0, PropValue::F32(f32::from_bits(0x7FC0_0001))));
        assert_ne!(nan_a, nan_b, "NaN bit patterns must hash by bits");
    }

    /// Golden digests for op_chain v1 (wire-fix regression before PROTOCOL_PIN).
    #[test]
    fn golden_v1_digests() {
        let spawn_op = spawn(10, None);
        assert_eq!(
            hex32(&digest_crdt_op(&spawn_op)),
            "d24fd679168eb0e1f3b30ee0d1be4d2486cb9b134f42bff9b3de66ac81fac27f"
        );

        let set_u32 = set_prop(9, 3, PropValue::U32(42));
        assert_eq!(
            hex32(&digest_crdt_op(&set_u32)),
            "2d66381f1e668151fd9db65c95e3d9da98cca0583dfe76d10f1448f6d19e9e92"
        );

        let zero = [0u8; 32];
        assert_eq!(
            hex32(&fold_op_chain(&zero, 1, 1, &spawn_op)),
            "a5d3f908ac3c541a865dd2a14a22f10af61481d273730957fae9bf0310c5dcf0"
        );

        let mut buf = Vec::new();
        assert_eq!(
            fold_op_chain_into(&zero, 1, 1, &spawn_op, &mut buf),
            fold_op_chain(&zero, 1, 1, &spawn_op)
        );
    }
}
