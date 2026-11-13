# x00a3 — MVCC×XPBD Phase 1 scaffold（着手）

> **ID:** `x00a3` · **slug:** `mvcc-phase1-scaffold` · **status:** `in_progress` · **area:** `[client, docs]` · **depends_on:** `[x00a0, x00a1, x00a2]`  
> **優先度:** [mvcc_xpbd.md](../mvcc_xpbd.md) Phase 1 着手（P1-2 / P1-3 吸収の土台）

目標: ゲート P1-1 / P1-4 / P1-5 closed 後、既存 `client/shared-sim` に **`mvcc`** モジュールを追加し、`HotPhysicsPage` / `MvccPageArena` の最小骨格と NaN 検疫・ゼロ初期化テストを載せる。現行 `EpochSheet` / `xpbd` 本線はまだ置換しない（段階共存）。

根拠: [improvement-plan-2026-11-12.md](../../0_doc/evaluation/improvement-plan-2026-11-12.md) · [mvcc_xpbd.md](../mvcc_xpbd.md) §0 / §4.3

---

## 保証する / しない（本スライス）

| 保証する | 保証しない |
|:---|:---|
| `HotPhysicsPage::zeroed_page` + `try_set_kinematic_pose` が NaN/Inf / inactive を拒否 | host/app 本線配線；`wide` 依存（人承認後） |
| `MvccPageArena::new` が固定プールを事前確保し、ページを checkout/commit できる最小 API | XOR デルタ；接触ソルバ置換；alloc 上限ベンチ全文 |
| `#![forbid(unsafe_code)]` 維持；bytemuck 既存依存のみ | Phase 2 wgpu / Phase 3 host 権威ループ |

## Phase

| Phase | 内容 | 状態 |
|:---|:---|:---|
| 0 | Slice + ゲート前提確認 | **done** |
| 1 | `mvcc/page.rs` + `arena.rs` + 単体テスト | **done** |
| 2 | XPBD 接触 / zero-alloc DoD（後続） | pending |
| 3 | host アダプタ境界（後続） | pending |

## 完了条件（本スライス = 着手）

1. `shared-sim` に `mvcc` 公開、ページ NaN 検疫テスト Green。
2. 現行 tape/xpbd 経路が壊れない（既存 shared-sim テスト Green）。
3. 本線置換・Phase 2 はしない。
