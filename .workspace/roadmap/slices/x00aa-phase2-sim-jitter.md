# x00aa — Phase 2 sim_tick jitter + PublishedPair 配線

> **ID:** `x00aa` · **slug:** `phase2-sim-jitter` · **status:** `done` · **area:** `[client, docs]` · **depends_on:** `[x00a9]`  
> **優先度:** [mvcc_xpbd.md](../mvcc_xpbd.md) Phase 2 / improvement-plan **P3-2**

目標: app が `PublishedPair` を消費し、補間 α を受信時刻基準から **`sim_tick` / `host_time_ms` + 1 frame jitter** に寄せる。Epoch 描画経路は並存（破壊しない）。

## 保証する / しない

| 保証する | 保証しない |
|:---|:---|
| Applied Frame 後に MVCC tip page が `sim_tick` 付きで publish | host `MvccPageArena` 本線 |
| jitter α ∈ [0,1]、窓外クランプ、単調 | proto 専用 `sim_tick` 欄（Phase 3；暫定は net `tick`） |
| Epoch 経路の既存テストが Green | 手元予測 / `interp.wgsl`（x00ab / x00ac） |

## 完了条件

1. `cargo test -p client sim_jitter` / `mvcc_mirror` / `epoch_mirror` Green。
2. roadmap 目次で本スライス `done`。
