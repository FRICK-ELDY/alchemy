# x00a9 — Phase 2 出版契約の土台（ArcSwap / sim_tick）

> **ID:** `x00a9` · **slug:** `phase2-publish-foundation` · **status:** `done` · **area:** `[client, docs]` · **depends_on:** `[x00a6, x00a7, x00a8]`  
> **優先度:** [mvcc_xpbd.md](../mvcc_xpbd.md) Phase 2 / improvement-plan **P3-2**（着手）

目標: Phase 2 ゲート（レイアウト・真 CoW・Directory・不等式）が Green になった前提で、`shared-sim` に **ArcSwap 出版ペア**（`sim_tick` 付き `PublishedFace`）を入れる。app の jitter / 手元予測 / `interp.wgsl` は後続サブスライス。

## 保証する / しない

| 保証する | 保証しない |
|:---|:---|
| `PublishedPair` が prev/curr を ArcSwap で公開 | wgpu Compute / naga 検証 |
| 単体で sim_tick 前進と pos 引き継ぎ | 手元予測 Keep/Blend/Snap |
| ゲート文書が「Phase 2 着手可」 | host dirty デルタ |

## 完了条件

1. `cargo test -p alchemy-shared-sim --lib mvcc::publish` Green。
2. roadmap 目次で本スライス `done`；次は app 側補間配線。
