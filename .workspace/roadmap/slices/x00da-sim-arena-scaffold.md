# x00da — Sim Arena scaffold（3 面骨格）

> **ID:** `x00da` · **slug:** `sim-arena-scaffold` · **status:** `done` · **area:** `[sim, client, docs]` · **depends_on:** `[]`  
> **優先度:** 品質本線 [scene_store_sim_arena.md](../scene_store_sim_arena.md) **Phase 1a** 着手  
> **後続:** Carry-Over は [x00db](x00db-sim-arena-carry-over.md)、本線配線は [x00dc](x00dc-sim-arena-bridge-publish.md)

目標: `client/shared-sim` に **`sim_arena/`** を新設し、`Prev` / `Published` / `Work` の 3 面ロールと最小 SoA（transform 列程度）、`LocalSlotId`、`STATE_TELEPORT_THIS_TICK` 定数を載せる。現行 `EpochSheet` / `FrameBridge` / host 本線はまだ置換しない。

根拠: [scene_store_sim_arena.md](../scene_store_sim_arena.md) §4 / §6 / §7 Phase 1a

---

## 保証する / しない（本スライス）

| 保証する | 保証しない |
|:---|:---|
| `sim_arena` 公開モジュールと 3 面ロールのローテーション API | `prev_dirty` / `prev2_dirty` Carry-Over 本実装（→ [x00db](x00db-sim-arena-carry-over.md)） |
| 最小 SoA（位置・回転など transform 列）と `LocalSlotId` | FrameBridge / XpbdWorld / host `sim.rs` 本線配線（→ [x00dc](x00dc-sim-arena-bridge-publish.md)） |
| `STATE_TELEPORT_THIS_TICK` 定数（§6 ビット割当） | bitmask edge Observer 本配線、Depth Bucket、物理 |
| `#![forbid(unsafe_code)]` 維持；新規外部クレートなし | scene_store / CRDT / SoftLease / `mvcc` 改称 |

## 触ったファイル

- `client/shared-sim/src/sim_arena/mod.rs` / `arena.rs`（新）
- `client/shared-sim/src/lib.rs`（`pub mod sim_arena`）

## Phase

| Phase | 内容 | 状態 |
|:---|:---|:---|
| 0 | Slice 文書 | **done** |
| 1 | `sim_arena` 骨格 + 面ローテ単体テスト | **done** |
| 2 | Carry-Over | → [x00db](x00db-sim-arena-carry-over.md) |

## 完了条件（本スライス）

1. `shared-sim` に `sim_arena` 公開、面ロール・ローテの単体テスト Green。
2. 現行 tape / xpbd / bridge / host / app 経路が無変更で壊れない。
3. Carry-Over・本線置換・CRDT・物理はしない。
