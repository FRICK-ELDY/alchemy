# x00e2 — SoftLease 編集ロック（Phase 1c 枠）

> **ID:** `x00e2` · **slug:** `soft-lease-edit-lock` · **status:** `planned` · **area:** `[host, sim]` · **depends_on:** `[x00e1]`  
> **優先度:** 品質本線 **Phase 1c**（1a/1b Green 後・DoD 必須外の枠）  
> **後続:** [x00e3](x00e3-trash-uuid-gc.md)

目標: host 入口に短命 **`SoftLease`** を置き、inspector / Reparent 等の同時構造編集奪い合いを防ぐ。物理掴み SSoT（`holder_slot` / `STATE_IS_GRABBED`）とは別概念で二重化しない。

根拠: [scene_store_sim_arena.md](../scene_store_sim_arena.md) §3.4 / §6 SoftLease / §7 Phase 1c

---

## 保証する / しない（本スライス）

| 保証する | 保証しない |
|:---|:---|
| SoftLease（holder_actor_id + lease_expires_tick）と edit ゲートでの競合拒否 | 物理 Grab の置換・`holder_slot` との統合 |
| 既存 Room Token `sub` 由来の actor 束縛を壊さない | TRASH / 3層 ID / `op_chain_hash` 必須化 |

## 完了条件

1. SoftLease 競合時に構造編集が拒否される最小テスト。
2. 物理 grab 経路を変更しない。
