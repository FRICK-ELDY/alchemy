# x00dc — Sim Arena ↔ FrameBridge / 既存出版（Phase 1a 閉じ）

> **ID:** `x00dc` · **slug:** `sim-arena-bridge-publish` · **status:** `done` · **area:** `[sim, host, client]` · **depends_on:** `[x00da, x00db]`  
> **優先度:** 品質本線 **Phase 1a DoD 閉じ**  
> **後続:** Scene Store は [x00dd](x00dd-scene-store-sparseset.md) 以降

目標: `sim_arena` の 3 面ローテ＋Carry-Over を既存 **`EpochSnapshotPair`（arc-swap）** 出版経路へ接続する。Phase 1a の Ring3 SSoT は host の **`start` / `world.curr`（f64 Epoch sheet）**（Arena 面の overlay はしない）。client `epoch_mirror` / `epoch_view` が壊れないこと。

根拠: [scene_store_sim_arena.md](../scene_store_sim_arena.md) §4.1 出版契約 / §7 Phase 1a

---

## 保証する / しない（本スライス）

| 保証する | 保証しない |
|:---|:---|
| FrameBridge および／または host `sim.rs` バリアが Arena ローテ＋Carry-Over 経由 | scene_store / LiveEditLedger facade（→ Phase 1b） |
| 既存 `EpochSnapshotPair` 1 store 出版を維持（Ring3 API 大破壊なし） | SoftLease / TRASH / StableUuid / `op_chain_hash` |
| bridge 経路でも先祖返りしないこと；rust-ci（client workspace test）Green | `mvcc/` → `sim_pages` 改称（→ [x00e7](x00e7-mvcc-rename-sim-pages.md)） |
| 出版は **caller sheet as-is**。Arena バリアは Carry-Over＋`rotate` を飛ばさない。**TELEPORT クリアは [x00db](x00db-sim-arena-carry-over.md)**（Work clear ＋ Published 離脱 clear） | 出版境界での TELEPORT 二重クリア／Arena→sheet overlay／独自クリア経路の新設 |
| 新規外部クレートなし | 物理ソルバ置換、Rapier |

## `STATE_TELEPORT_THIS_TICK`（本スライス）

- **クリア本体は x00db**（Step 0 の Work clear ＋ `rotate_faces` の Published 離脱 clear）。本スライスはバリア順だけを保証する。
- **必須順序**: `Carry-Over(+ Work TELEPORT clear) → writers（`write_*`）→ **publish（caller Epoch sheets）→ rotate`。
  - Published 離脱 clear は面バッファを **in-place** 更新する。Ring3 へ live 面を跨 rotate 共有してはならない（owned sheet / arc-swap のみ）。
- Work 書き込みは **`SimArena::write_*` のみ**（本線で raw `work_mut` を呼ばない）。
- **Phase 1a**: TELEPORT の観測・寿命は **Arena 面のみ**。`EpochSheet` / Ring3 GPU 経路にはまだ載せない（将来 GPU snap は別スライス）。

## 触ったファイル

- `client/shared-sim/src/bridge.rs` — `publish_arena_then_rotate`（**sheet を as-is 出版**、Arena overlay なし）/ seed / dirty sync + bridge 先祖返り・stale-Work 回帰テスト
- `client/shared-sim/src/xpbd.rs` — 出版順コメント（最小）
- `client/shared-sim/src/sim_arena/*` — `seed_slot` + モジュール配線コメント
- `client/shared-sim/src/lib.rs` — re-export
- `client/host-server/src/sim.rs` — Arena バリア（carry → sync → publish sheet → rotate）；`sync_page_poses_to_sheet` が pose 変化で `mark_dirty`；host/mvcc DoD テスト
- `client/app` epoch_mirror / epoch_view — 変更なし（`EpochSnapshotPair` 契約維持）

## Ring3 SSoT（Phase 1a 確定）

**正は host の `start` / `world.curr`（f64 Epoch sheet）。** Arena は Carry-Over / TELEPORT 寿命用の補助。Arena f32 面を Ring3 sheet に overlay しない（MvccXpbd の page→sheet ずれで古い Work が curr を潰すのを防ぐ）。

## Phase

| Phase | 内容 | 状態 |
|:---|:---|:---|
| 0 | Slice 文書 | **done** |
| 1 | shared-sim bridge ↔ Arena 接続（x00db Step 0 順守） | **done** |
| 2 | host / client 消費側確認 + Phase 1a DoD | **done** |

## 完了条件（本スライス = Phase 1a 閉じ）

1. 先祖返りテストが bridge（または host バリア）経路でも成立。
2. バリアが x00db の Carry-Over＋TELEPORT 寿命（Work clear / Published 離脱 clear）を経由し、**publish → rotate** 順を守ること（スキップ経路・live 跨 rotate を作らない）。
3. `cargo test --workspace`（`client/`）相当の rust-ci Green。新規クレートなし。
4. scene_store・LiveEditLedger 改修・`mvcc` 改称はしない。
