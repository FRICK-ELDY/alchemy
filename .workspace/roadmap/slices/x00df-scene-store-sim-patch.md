# x00df — SimPatch キュー → Arena Drain

> **ID:** `x00df` · **slug:** `scene-store-sim-patch` · **status:** `done` · **area:** `[sim, client]` · **depends_on:** `[x00de, x00dc]`  
> **優先度:** 品質本線 Phase 1b（Ring1→Ring2 合流）  
> **後続:** LiveEditLedger facade は [x00e0](x00e0-live-edit-facade.md)

目標: Scene Store から **`SimPatch` キュー**を生成し、Sim Arena の Step 0 で一括 Drain して Work 面へ適用する（ページスワップ禁止・Lost Update 防止）。

根拠: [scene_store_sim_arena.md](../scene_store_sim_arena.md) §4.2 / §6 `SimPatch` / §7 Phase 1b

---

## 保証する / しない（本スライス）

| 保証する | 保証しない |
|:---|:---|
| `SimPatch` enum（Allocate/Free/SetParent/SetLocalTransform/bits 等）とキュー Drain | host Zenoh / Save I/O |
| Tick 境界一括適用；Lost Update なし合流テスト | SoftLease、ドラッグ揮発ストリーム本線、Depth Bucket 本実装必須化 |
| Arena 既存 Carry-Over 契約を壊さない | LiveEditLedger 第二コピー解消（→ [x00e0](x00e0-live-edit-facade.md)） |

## 触ったファイル

- `client/shared-sim/src/scene_store/patch.rs` — `SimPatch` enum（新）
- `client/shared-sim/src/scene_store/store.rs` — キュー生成（Spawn/Move）・`enqueue_patch` / `drain_patches`
- `client/shared-sim/src/scene_store/mod.rs` / `lib.rs` — 配線・re-export
- `client/shared-sim/src/sim_arena/arena.rs` — `apply_patches` / `step0_carry_and_apply_patches` + durable カラム
- `client/shared-sim/src/sim_arena/mod.rs` — Step 0 順コメント
- host facade / edit ワイヤ — **変更なし**

## Phase

| Phase | 内容 | 状態 |
|:---|:---|:---|
| 0 | Slice 文書 | **done** |
| 1 | `SimPatch` + Scene Store キュー + Arena Drain | **done** |
| 2 | 同一 slot 連続 patch 最終値テスト Green | **done** |

## 完了条件

1. Patch 合流の単体テスト Green（同一 slot への連続 patch が最終値になる）。
2. host facade 未着手。
