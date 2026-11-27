# x00e5 — op_chain_hash 口 ＋ RoomSnapshot 枠

> **ID:** `x00e5` · **slug:** `op-chain-room-snapshot` · **status:** `done` · **area:** `[sim, host, world]` · **depends_on:** `[x00e1]`  
> **優先度:** 品質本線 Phase 1c（枠のみ・1a/1b DoD 外）  
> **後続:** [x00e6](x00e6-pending-local-ops.md)

目標: `SequencedOp.op_chain_hash` の更新口と Late Join 用 **RoomSnapshot**（Scene Store 全量＋実行面）の枠を置く。`World.Verifier` Behaviour への将来入力を意識し、Spot-Check 統計は入れない。

根拠: [scene_store_sim_arena.md](../scene_store_sim_arena.md) §5.2 / §5.3 / §7 Phase 1c

---

## 保証する / しない（本スライス）

| 保証する | 保証しない |
|:---|:---|
| `op_chain_hash` 更新フックと RoomSnapshot データ形の枠 | Verifier 本番統計・Proprietary Spot-Check |
| 現行 `…/snapshot` / `…/sim_state` 契約を無断弱体化しない | 欠損厳禁 Reliable Op トランスポート全面置換 |

## 完了条件

1. 連鎖ハッシュ更新口と RoomSnapshot 骨格（テストまたは型のみ可）。
2. OSS に Proprietary ヒューリスティックを混入しない。

## 実装メモ（done）

- [`client/shared-sim/src/scene_store/op_chain.rs`](../../../client/shared-sim/src/scene_store/op_chain.rs): `SequencedOp`；`OP_CHAIN_DOMAIN`；`digest_crdt_op` / `_into` / `fold_op_chain*`（blake3 via `alchemy_protocol::blake3_hash_32`；ゴールデン v1）。
- [`client/shared-sim/src/scene_store/room_snapshot.rs`](../../../client/shared-sim/src/scene_store/room_snapshot.rs): `RoomSnapshot`；`from_scene_store` は **entity_id 昇順** + props 昇順。TrashStore TTL/FIFO メタは非保証。
- `SceneStore::properties` — prop_key 昇順の決定論 export。
- [`client/host-server/src/live_edit.rs`](../../../client/host-server/src/live_edit.rs): `op_chain_hash` + `fold_confirmed_op` → `SequencedOp`（tip bump **後**；`debug_assert!(tip > 0)`）。
- **PROTOCOL_PIN / EditCommit / `…/snapshot` / `…/sim_state` は変更なし**。
- **host_loop の tip 経路へ auto-fold は未配線**（枠；Reliable Op / Verifier 入力時に接続）。
- Spot-Check 統計・閾値は置かない（OSS Behaviour 注入点のみ維持）。
