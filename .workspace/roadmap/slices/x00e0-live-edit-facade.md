# x00e0 — LiveEditLedger → scene_store facade

> **ID:** `x00e0` · **slug:** `live-edit-facade` · **status:** `done` · **area:** `[host, sim]` · **depends_on:** `[x00df]`  
> **優先度:** 品質本線 Phase 1b（疎 SSoT 一本化）  
> **後続:** Undo / pending_despawns 閉じは [x00e1](x00e1-scene-store-undo-despawn.md)

目標: host [`LiveEditLedger`](../../client/host-server/src/live_edit.rs) を **薄い facade** にする。親子・fields・entities の正は `shared-sim::scene_store`。host に残すのは **検疫・`scene_rev`（tip）採番・edit/Save I/O**。疎状態の第二コピー禁止。

根拠: [scene_store_sim_arena.md](../scene_store_sim_arena.md) モジュール配置 / §7 Phase 1b / Non-Negotiable §8.6

---

## 保証する / しない（本スライス）

| 保証する | 保証しない |
|:---|:---|
| parents / fields / entities の正が scene_store；host に同型 HashMap 二重保持なし | tip 採番・AEAD・JWT `sub` Dual-auth・Save CAS の現行ゲート弱体化 |
| edit 経路が facade 経由で Reparent / SetField を適用 | TRASH_UUID、SoftLease、StableUuid、`op_chain_hash` |
| 既存 reject 理由（CYCLE / STALE 等）の契約維持 | SlotUndoStack 本接続（→ [x00e1](x00e1-scene-store-undo-despawn.md)） |

## 触るファイル（予定）

- `client/host-server/src/live_edit.rs` / `edit.rs` / 必要なら `host_loop.rs`
- `client/shared-sim/src/scene_store/*`

## 完了条件

1. facade 経由の Reparent / SetField が動作；第二コピーなし。
2. 認証・検証スキップなし。既存 edit テストの意図を維持または更新。
