# x00dc — Sim Arena ↔ FrameBridge / 既存出版（Phase 1a 閉じ）

> **ID:** `x00dc` · **slug:** `sim-arena-bridge-publish` · **status:** `planned` · **area:** `[sim, host, client]` · **depends_on:** `[x00da, x00db]`  
> **優先度:** 品質本線 **Phase 1a DoD 閉じ**  
> **後続:** Scene Store は [x00dd](x00dd-scene-store-sparseset.md) 以降

目標: `sim_arena` の 3 面ローテ＋Carry-Over を既存 **`EpochSnapshotPair`（arc-swap）** 出版経路へ接続する。Ring3 契約は維持（`prev` = Arena.Prev、`curr` = Arena.Published）。client `epoch_mirror` / `epoch_view` が壊れないこと。

根拠: [scene_store_sim_arena.md](../scene_store_sim_arena.md) §4.1 出版契約 / §7 Phase 1a

---

## 保証する / しない（本スライス）

| 保証する | 保証しない |
|:---|:---|
| FrameBridge および／または host `sim.rs` バリアが Arena ローテ＋Carry-Over 経由 | scene_store / LiveEditLedger facade（→ Phase 1b） |
| 既存 `EpochSnapshotPair` 1 store 出版を維持（Ring3 API 大破壊なし） | SoftLease / TRASH / StableUuid / `op_chain_hash` |
| bridge 経路でも先祖返りしないこと；rust-ci（client workspace test）Green | `mvcc/` → `sim_pages` 改称（→ [x00e7](x00e7-mvcc-rename-sim-pages.md)） |
| 出版は Prev/Published **as-is**（TELEPORT 含む）。**クリアは [x00db](x00db-sim-arena-carry-over.md)**（Step 0 Work clear ＋ rotate の Published 離脱 clear）を配線で飛ばさない | 出版境界での TELEPORT 二重クリア／独自クリア経路の新設 |
| 新規外部クレートなし | 物理ソルバ置換、Rapier |

## `STATE_TELEPORT_THIS_TICK`（本スライス）

- **クリア本体は x00db**（Step 0 の Work clear ＋ `rotate_faces` の Published 離脱 clear）。本スライスはバリア順だけを保証する。
- **必須順序**: `Carry-Over(+ Work TELEPORT clear) → writers（`write_*`）→ **publish（Prev/Published スナップショット）→ rotate`。
  - Published 離脱 clear は面バッファを **in-place** 更新する。`publish` より後・または Ring3 が同一面メモリを rotate またぎで保持すると、読取側から TP が消える。
  - 出版はコピーまたは arc-swap で **rotate 前の内容**を渡すこと。live 参照の跨 rotate 共有は禁止。
- Work 書き込みは **`SimArena::write_*` のみ**（本線で raw `work_mut` を呼ばない）。`work_mut`＋`mark_dirty` なしは dirty 窓から外れ、Carry-Over **と** Published 離脱 clear の両方の対象外になり TELEPORT 蘇り経路が残る。型／CI で raw を封じるのは任意（現状は契約＋レビューで縛る）。
- Ring3（GPU）は Published 面の TELEPORT を読んで 1 世代スナップすればよい。

## 触るファイル（予定）

- `client/shared-sim/src/bridge.rs` / `xpbd.rs`（必要最小）
- `client/host-server/src/sim.rs`（バリア）
- `client/app/src/epoch_mirror.rs` / `epoch_view.rs`（消費側が壊れない範囲）
- `client/shared-sim/src/sim_arena/*`（配線のみ；TELEPORT clear ロジック追加は x00db）

## Phase

| Phase | 内容 | 状態 |
|:---|:---|:---|
| 0 | Slice 文書 | planned |
| 1 | shared-sim bridge ↔ Arena 接続（x00db Step 0 順守） | planned |
| 2 | host / client 消費側確認 + Phase 1a DoD | planned |

## 完了条件（本スライス = Phase 1a 閉じ）

1. 先祖返りテストが bridge（または host バリア）経路でも成立。
2. バリアが x00db の Carry-Over＋TELEPORT 寿命（Work clear / Published 離脱 clear）を経由し、**publish → rotate** 順を守ること（スキップ経路・live 跨 rotate を作らない）。
3. `cargo test --workspace`（`client/`）相当の rust-ci Green。新規クレートなし。
4. scene_store・LiveEditLedger 改修・`mvcc` 改称はしない。
