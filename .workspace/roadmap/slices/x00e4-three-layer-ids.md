# x00e4 — 3層 ID（StableUuid / WireSlotId / LocalSlotId）

> **ID:** `x00e4` · **slug:** `three-layer-ids` · **status:** `planned` · **area:** `[sim, protocol, host, client]` · **depends_on:** `[x00e1]`  
> **優先度:** 品質本線 Phase 1c  
> **後続:** [x00e5](x00e5-op-chain-room-snapshot.md)

目標: ワイヤ／永続に `WireSlotId` 配布と `StableUuid` を導入し、内部は `wire_to_local` で O(1)。一気に全経路 StableUuid 化しない（段階移行）。

根拠: [scene_store_sim_arena.md](../scene_store_sim_arena.md) §3.1 / §6 型定義 / §7 Phase 1c

---

## 保証する / しない（本スライス）

| 保証する | 保証しない |
|:---|:---|
| 3層 ID 型と `wire_to_local` 骨格；移行パス文書 | 全ワイヤ・全 Save の一括 StableUuid 切替 |
| 1b の `entity_id: u32` 経路との共存期間を明示 | PendingLocalOps、物理 |

## 完了条件

1. 型・マップの最小実装と移行テスト。
2. PROTOCOL_PIN / 既存 edit 契約を無断破壊しない（必要なら別プロトコルスライス）。
