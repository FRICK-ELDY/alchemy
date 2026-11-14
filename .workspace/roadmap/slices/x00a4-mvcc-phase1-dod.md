# x00a4 — MVCC×XPBD Phase 1 DoD（接触 / zero-alloc / リプレイ）

> **ID:** `x00a4` · **slug:** `mvcc-phase1-dod` · **status:** `done` · **area:** `[client, docs]` · **depends_on:** `[x00a3]`  
> **優先度:** [mvcc_xpbd.md](../mvcc_xpbd.md) Phase 1 DoD（P1-2 / P1-3 吸収）

目標: x00a3 scaffold の上で、ページ／アリーナ経路の **body-body 接触**・**プール page-alloc 上限**・**ビット一致リプレイ**・**`frame_delta` 速度復元** を `shared-sim` 単体テストで閉じる。host/app 本線は触らない。

根拠: [improvement-plan-2026-11-12.md](../../0_doc/evaluation/improvement-plan-2026-11-12.md) · [mvcc_xpbd.md](../mvcc_xpbd.md) §5 Phase 1 · 計画「MVCC×XPBD — 次にやるべきこと」

---

## 保証する / しない（本スライス）

| 保証する | 保証しない |
|:---|:---|
| `PageXpbdAdapter` が `HotPhysicsPage` 上で LBVH 接触付き `xpbd::step` を回す | host/app 配線・`ALCHEMY_SIM_BACKEND` |
| 2 箱フィクスチャがページ経路で重ならない（P1-2） | `wide` SIMD（人承認後） |
| **P1-3**: 1k entity・128 Tick 履歴・500 Tick でプール枯渇なし・**tick page-alloc ≤ 2**（プール外ページ確保のみ） | `WorldSnapshot` の `Vec`、XPBD scratch/`FlatLbvh`、シート clone など **非 page ヒープ** |
| Tick 50→100 および `snapshot_at` 巻き戻し再シミュが SoA + **全 mask（dirty 含む）** ビット一致 | Phase 2 wgpu / Phase 3 XOR デルタ |
| `pos - frame_delta` を `step_page` の pull で復元した 1 Tick が連続実行と一致 | 大規模 CCU / Spatial Interest |

## Phase

| Phase | 内容 | 状態 |
|:---|:---|:---|
| 0 | Slice + x00a3 前提 | **done** |
| 1 | `fork_page` / `commit_transfer`·`commit_share`（retain→release）/ tick alloc カウンタ | **done** |
| 2 | `PageXpbdAdapter` + 接触 / alloc / リプレイ / delta テスト | **done** |

## 完了条件

1. `cargo test -p alchemy-shared-sim --lib mvcc` Green（接触・alloc・リプレイ・delta 含む）。
2. Epoch / host 本線に差分なし（置換しない）。
3. x00a3 Phase 2（接触 / zero-alloc）を本スライスで吸収して閉じる。
