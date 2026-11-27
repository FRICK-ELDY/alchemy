# x00e1 — scene_store Undo 接続 ＋ pending_despawns（Phase 1b 閉じ）

> **ID:** `x00e1` · **slug:** `scene-store-undo-despawn` · **status:** `done` · **area:** `[host, sim, client]` · **depends_on:** `[x00e0]`  
> **優先度:** 品質本線 **Phase 1b DoD 閉じ**  
> **後続:** Phase 1c 枠は [x00e2](x00e2-soft-lease-edit-lock.md) 以降

目標: 既存 [`SlotUndoStack`](../../client/shared-sim/src/slot_undo.rs) を scene_store 親へ接続し、削除は現行 **`pending_despawns`** を維持したまま Phase 1b を閉じる。

根拠: [scene_store_sim_arena.md](../scene_store_sim_arena.md) §3.2 / §3.3 / §7 Phase 1b DoD

---

## 保証する / しない（本スライス）

| 保証する | 保証しない |
|:---|:---|
| facade 経由 Reparent / Despawn（pending 系）と Undo 接続の最小経路 | `TRASH_UUID` / Trash GC（→ [x00e3](x00e3-trash-uuid-gc.md)） |
| 循環 Reject・LWW・既存 edit ゲート（`sub` / ManifestSeal 経路）維持 | SoftLease / StableUuid / `op_chain_hash` / PendingLocalOps |
| Phase 1b DoD Green | Phase 1c / 2 / 3 |

## 触るファイル（予定）

- `client/shared-sim/src/slot_undo.rs` / `scene_store/*`
- `client/host-server/src/live_edit.rs` / despawn 経路

## 完了条件（Phase 1b 閉じ）

1. 循環 Reject・LWW・facade 経由 Reparent / Despawn（pending）テスト Green。
2. TRASH / SoftLease / StableUuid / `op_chain_hash` を入れない。
