# x00db — Sim Arena 2-Frame Dirty Carry-Over

> **ID:** `x00db` · **slug:** `sim-arena-carry-over` · **status:** `planned` · **area:** `[sim, client]` · **depends_on:** `[x00da]`  
> **優先度:** 品質本線 Phase 1a（先祖返り防止契約）  
> **後続:** bridge / host 配線は [x00dc](x00dc-sim-arena-bridge-publish.md)

目標: [x00da](x00da-sim-arena-scaffold.md) の `sim_arena` に **`prev_dirty_slots` + `prev2_dirty_slots`** と Tick 冒頭の `Work[slot] = Published[slot]` 引き継ぎ、カラム別 `dirty_epoch` を実装する。FrameBridge / host 本線はまだ置換しない（ライブラリ単体で DoD）。

根拠: [scene_store_sim_arena.md](../scene_store_sim_arena.md) §4.1 / §7 Phase 1a DoD

---

## 保証する / しない（本スライス）

| 保証する | 保証しない |
|:---|:---|
| 動いた行だけ Carry-Over；静止多数のフルコピーなし | FrameBridge / `EpochSnapshotPair` 本線置換（→ [x00dc](x00dc-sim-arena-bridge-publish.md)） |
| 「動かす → 静止 → 3 tick」で座標が 2 フレーム前へ先祖返りしない単体テスト | scene_store / `SimPatch` Drain / 物理 |
| カラム別 `dirty_epoch: u64` | 30Hz dirty ワイヤ変更、RenderFrame 契約変更 |
| **`STATE_TELEPORT_THIS_TICK` の Work クリア**（Step 0 / Carry-Over 後。recycle だけでは消えない） | Observer 本体の実装、出版境界での二重クリア（→ [x00dc](x00dc-sim-arena-bridge-publish.md) は Published をそのまま出す） |
| 新規外部クレートなし | SoftLease / CRDT / `mvcc`→`sim_pages` |

## `STATE_TELEPORT_THIS_TICK`（本スライスの責務）

| 段階 | 振る舞い |
|:---|:---|
| `rotate_faces` | ビットは**残る**（面の役割だけ入れ替え） |
| Step 0 Carry-Over | `Work[slot] = Published[slot]` で **pos + rot + state_bits 等を行コピー**したあと、**Work 上の TELEPORT ビットを明示 clear** |
| その後の writer | 今 tick だけ TELEPORT を立て直せる |
| Published | Ring3 用に直前 tick の TELEPORT を保持（次の rotate まで） |

## 触るファイル（予定）

- `client/shared-sim/src/sim_arena/*`（Carry-Over・`dirty_epoch`・TELEPORT Work clear）
  - 行コピー対象: **`pos_*` に加え `rot_*`（x00da は列確保のみ・読取 API なし）と `state_bits`**。同モジュール private 参照で rot も漏れなく copy
- 単体テスト（同モジュールまたは `tests`）

## Phase

| Phase | 内容 | 状態 |
|:---|:---|:---|
| 0 | Slice 文書 | planned |
| 1 | prev/prev2 dirty + Carry-Over（pos+**rot**+bits）+ dirty_epoch + TELEPORT Work clear | planned |
| 2 | 先祖返り 3 tick 単体テスト Green | planned |

## 完了条件（本スライス）

1. Carry-Over 実装と「静止後 3 tick で先祖返りしない」単体テスト Green。
2. Carry-Over 後に Work の `STATE_TELEPORT_THIS_TICK` が落ちている単体テスト（Published 側は直前値を保持しうる）。
3. `cargo test -p alchemy-shared-sim` Green。
4. host / app / FrameBridge 本線は未配線のまま（本線閉じは [x00dc](x00dc-sim-arena-bridge-publish.md)）。
