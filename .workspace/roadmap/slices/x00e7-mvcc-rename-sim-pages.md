# x00e7 — mvcc/ → sim_pages/ 改称

> **ID:** `x00e7` · **slug:** `mvcc-rename-sim-pages` · **status:** `planned` · **area:** `[sim, docs]` · **depends_on:** `[x00da]`  
> **優先度:** 品質本線 Non-Negotiable §8.7（1a 後推奨・いつでも可）  
> **並行可:** Phase 1b と独立してよい（品質 SSoT の座を奪わない）

目標: `client/shared-sim/src/mvcc/` を **`sim_pages/`** へ改称し、ドキュメント・`lib.rs`・参照を更新する。ページプールは **opt-in / 学習資産**のまま。実行面 SSoT は `sim_arena`、疎 SSoT は `scene_store`。

根拠: [scene_store_sim_arena.md](../scene_store_sim_arena.md) モジュール改称方針 / §8.7

---

## 保証する / しない（本スライス）

| 保証する | 保証しない |
|:---|:---|
| モジュールパス改称とコンパイル・テスト Green | Arena / scene_store 機能追加 |
| ドキュメント上の `sim_pages` 用語統一 | `HISTORY_TICKS=128` ページ CoW 本線復帰 |

## 触るファイル（予定）

- `client/shared-sim/src/mvcc/` → `sim_pages/`
- 参照元・roadmap 文言

## 完了条件

1. `pub mod sim_pages`；旧 `mvcc` パスなし（または薄い互換 re-export を明示期限付き）。
2. `cargo test -p alchemy-shared-sim` Green。
