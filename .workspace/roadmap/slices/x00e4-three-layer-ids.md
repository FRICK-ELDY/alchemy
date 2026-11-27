# x00e4 — 3層 ID（StableUuid / WireSlotId / LocalSlotId）

> **ID:** `x00e4` · **slug:** `three-layer-ids` · **status:** `done` · **area:** `[sim, protocol, host, client]` · **depends_on:** `[x00e1]`  
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

## 実装メモ（done）

- [`client/shared-sim/src/ids.rs`](../../../client/shared-sim/src/ids.rs): `StableUuid` / `WireSlotId` / `LocalSlotId` / `ROOT_UUID` / `SLOT_ABSENT` / `TRASH_*`；[`WireToLocal`]（O(1) `wire→local` + reverse + Stable 台帳 HashMap）；1b 共存ヘルパ（`stable_from_entity_id` / `bind_identity` 等）。
- `LocalSlotId` 定義を `ids` へ集約；`sim_arena` / `SimPatch` / bridge は再エクスポート／`crate::ids` 参照。
- `TRASH_UUID` / `TRASH_ENTITY_ID` 定義を `ids` へ移し、`scene_store::trash` は再エクスポート。`SparseSet` ABSENT = `ids::SLOT_ABSENT`。
- bind は `SLOT_ABSENT` / `TRASH_ENTITY_ID` 拒否；forward/reverse 不一致は `Inconsistent`（自動修復なし）；`StableTaken` vs `StableConflict` を分離。
- Vec grow は SparseSet と同様 **host/quarantine-bounded**（巨大 id で OOM しうる — 数値天井はゲート側）。
- **PROTOCOL_PIN / EditProposal `entity_id: u32` / CrdtOp キーは変更なし**（別プロトコルスライスで切替）。
- **`WireToLocal` は host / SceneStore 未配線**（骨格のみ。配布・remap が必要になった時点で保持）。

## 1b → 1c 移行パス（entity_id 共存）

| 段階 | ライブ SparseSet / edit ワイヤ | 内部 Arena | 永続 / VCS |
|:---|:---|:---|:---|
| **1b（現行維持）** | `entity_id: u32` | `LocalSlotId == entity_id`（数値同一） | Manifest / Save も `entity_id` |
| **1c（本スライス）** | **同上**（PROTOCOL_PIN 無断破壊なし） | [`WireToLocal`] 骨格；1b ノードは `bind_identity` | `StableUuid` 型＋ embed ヘルパのみ（Save スキーマ未切替） |
| **後続（別スライス）** | Spawn に `wire_slot` + `StableUuid`；pin bump | wire ≠ local を許可し remap；**host が `WireToLocal` を保持** | SparseSet / Save キーを StableUuid へ |

ライブ経路は当面 `EntityId` のまま。`WireToLocal` は加算的骨格（x00e5 の `op_chain_hash` / RoomSnapshot 枠とも独立；配線は WireSlotId 配布が必要になったスライスで）。
