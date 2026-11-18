# x00d8 — LBVH O(n) + サブステップ refit（P0-2）

> **ID:** `x00d8` · **slug:** `lbvh-o-n-refit` · **status:** `done` · **area:** `[shared-sim, docs]` · **depends_on:** `[x008e, x00a2, x00ad]`  
> **優先度:** improvement-plan 2026-11-18 **P0-2**

目標: Flat LBVH 構築を `(morton, leaf_index)` 添字引きで **O(n)** にし、XPBD tick 内は第 1 サブステップのみ rebuild・以降は **refit**。SceneBudget 上限（4096）と権威物理 1 tick 時間予算を同じ数字で文書化する。

根拠: [improvement-plan-2026-11-18.md](../../0_doc/evaluation/improvement-plan-2026-11-18.md) P0-2 · [specific-weaknesses-2026-11-18.md](../../0_doc/evaluation/specific-weaknesses-2026-11-18.md) LBVH O(n²) -2

---

## 保証する / しない

| 保証する | 保証しない |
|:---|:---|
| `FlatLbvh::rebuild_from_sheet` は Morton + radix + 葉 AABB **添字引き**で構築 O(n) | Karras 並列構築；正確 OBB |
| `xpbd::step(..., bounds: Some)` は **substep 0 = rebuild**、以降は **refit**（葉数 / readiness 不一致時は同サブステップで rebuild）。`refit_from_sheet` が `false` のとき木は**未変更**（検証完了後にのみ葉 AABB を書く）。現行の本番呼び出しは `xpbd::step` のみ | フレーム跨ぎの Morton 順最適性（毎 tick 先頭 rebuild に依存）；`refit` 戻り値を無視した外部呼び出し |
| SceneBudget **entries ≤ 4096**（x00a2）を host LBVH 計測天井とする。権威 tick **60 Hz / 16.6 ms** 内で LBVH は **1 rebuild + (substeps−1) refit** | CI 上の厳密 wall-clock SLA；構築 < 0.2 ms の Enforce（設計目標・`#[ignore]` 計測） |
| `lbvh_build_timing_1024_4096`（`#[ignore]`）で n=1024/4096 の rebuild/refit を計測可能 | Epoch sheet clone / 確保削減（P1-1） |

---

## Phase

| Phase | 内容 | 状態 |
|:---|:---|:---|
| 0 | Slice + 目次 | **done** |
| 1 | `(code, leaf_index)` O(n) rebuild + capacity reuse | **done** |
| 2 | `refit_from_sheet` + `XpbdWorld.lbvh_scratch` 配線 | **done** |
| 3 | 回帰 / `#[ignore]` 計測 + architecture 保証 | **done** |

## 完了条件

1. 構築に葉 AABB の線形 `find` が無い（添字引き）。
2. 既定 4 substeps で 1 rebuild + 3 refit（トポロジ変更時はフォールバック rebuild）。
3. `cargo test -p alchemy-shared-sim --lib` 緑；`lbvh_build_timing_1024_4096 --ignored` で計測可能。
4. `client.md` / `spatial.rs` 保証コメントが本 Slice と一致。

## 実装メモ

- コード: `client/shared-sim/src/spatial.rs` · `client/shared-sim/src/xpbd.rs`
- 設計目標（aspirational）: [alchemy_zero_base_spatial_lifecycle_architecture.md](../alchemy_zero_base_spatial_lifecycle_architecture.md) §4.2 数千オブジェクトで構築 < 0.2 ms
