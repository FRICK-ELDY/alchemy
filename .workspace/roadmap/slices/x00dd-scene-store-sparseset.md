# x00dd — scene_store SparseSet 骨格

> **ID:** `x00dd` · **slug:** `scene-store-sparseset` · **status:** `done` · **area:** `[sim, client]` · **depends_on:** `[x00dc]`  
> **優先度:** 品質本線 **Phase 1b** 着手  
> **後続:** LWW / 循環は [x00de](x00de-scene-store-lww-cycle.md)

目標: `client/shared-sim` に **`scene_store/`** を新設し、型付き `SparseSet<T>`（sparse / dense_slots / dense_values）と **sparse キー = `entity_id: u32`** の骨格を載せる。host `LiveEditLedger` はまだ facade 化しない。

根拠: [scene_store_sim_arena.md](../scene_store_sim_arena.md) §3.1 / §3.3 / §7 Phase 1b

---

## 保証する / しない（本スライス）

| 保証する | 保証しない |
|:---|:---|
| `SparseSet<T>` と dense 走査・穴許容 | Host-Sequenced LWW / 循環防止（→ [x00de](x00de-scene-store-lww-cycle.md)） |
| sparse キー = `entity_id: u32`（StableUuid はまだ）。**`entity_id == u32::MAX` 禁止**（sparse ABSENT 予約；insert は no-op） | LiveEditLedger facade、Zenoh edit、Save |
| `#![forbid(unsafe_code)]`；新規クレートなし | TRASH_UUID / SoftLease / 3層 ID |

## 触ったファイル

- `client/shared-sim/src/scene_store/mod.rs` / `sparse_set.rs`（新）
- `client/shared-sim/src/lib.rs`（`pub mod scene_store` + re-export）

## Phase

| Phase | 内容 | 状態 |
|:---|:---|:---|
| 0 | Slice 文書 | **done** |
| 1 | `SparseSet` insert / remove / dense 走査（穴許容） | **done** |
| 2 | LWW / 循環 | → [x00de](x00de-scene-store-lww-cycle.md) |

## 完了条件

1. `SparseSet` の insert / remove / dense 走査テスト Green（穴許容）。
2. host / edit ワイヤ未変更。
