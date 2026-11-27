# x00e7 — mvcc/ → sim_pages/ 改称

> **ID:** `x00e7` · **slug:** `mvcc-rename-sim-pages` · **status:** `done` · **area:** `[sim, docs]` · **depends_on:** `[x00da]`  
> **優先度:** 品質本線 Non-Negotiable §8.7（1a 後推奨・いつでも可）  
> **並行可:** Phase 1b と独立してよい（品質 SSoT の座を奪わない）

目標: `client/shared-sim/src/mvcc/` を **`sim_pages/`** へ改称し、ドキュメント・`lib.rs`・参照を更新する。ページプールは **opt-in / 学習資産**のまま。実行面 SSoT は `sim_arena`、疎 SSoT は `scene_store`。

根拠: [scene_store_sim_arena.md](../scene_store_sim_arena.md) モジュール改称方針 / §8.7

**スコープ（意図）**: 本スライスは **Rust モジュールパス**（`pub mod sim_pages` / `crate::sim_pages` / `alchemy_shared_sim::sim_pages`）のみ。型名・フィールド名・app ファイル名（`MvccPageArena` / `HostMvccState` / `PhysicsWorld.mvcc` / `mvcc_mirror` / `SimBackendKind::MvccXpbd` / `"mvcc_xpbd"`）は**意図的に据え置き**（後続任意。DoD の「旧 `mvcc` パスなし」はモジュールパスを指す）。

---

## 保証する / しない（本スライス）

| 保証する | 保証しない |
|:---|:---|
| モジュールパス改称とコンパイル・テスト Green | Arena / scene_store 機能追加 |
| ドキュメント上の `sim_pages` 用語統一（ライブ参照） | `HISTORY_TICKS=128` ページ CoW 本線復帰 |
| | 型・フィールド・`mvcc_mirror` / `mvcc_xpbd` 識別子の改称 |

## 触ったファイル

- `client/shared-sim/src/mvcc/` → `sim_pages/`（8 ファイル）
- `client/shared-sim/src/lib.rs` — `pub mod sim_pages`；crate docs
- `client/shared-sim/src/delta_codec.rs` / `sim_arena/mod.rs` / `sim_pages/*` — `crate::sim_pages` 参照
- `client/host-server/src/sim.rs` / `network_worker.rs` — `alchemy_shared_sim::sim_pages`
- `client/app/src/mvcc_mirror.rs` / `bus/frame.rs` / `render_wgpu/*`（`interp.wgsl` 含む）— 同上
- [scene_store_sim_arena.md](../scene_store_sim_arena.md) / [README.md](../README.md) / improvement-plan P3-8
- リンク修正: [x00a5](x00a5-soa-safe-simd.md) inertia パス、[x00c0](x00c0-host-cell-dirty-filter.md) `CellId`

## 完了条件

1. `pub mod sim_pages`；旧 `mvcc` **モジュール**パスなし（互換 re-export なし）。
2. `cargo test -p alchemy-shared-sim` Green。
