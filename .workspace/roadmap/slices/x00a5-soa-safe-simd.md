# x00a5 — SoA ホットパス Safe SIMD（`wide` / `glam`）

> **ID:** `x00a5` · **slug:** `soa-safe-simd` · **status:** `done` · **area:** `[client, docs]` · **depends_on:** `[x00a4]`  
> **優先度:** [mvcc_xpbd.md](../mvcc_xpbd.md) §6 Q5 承認済み後続強化（Phase 1 DoD 後・host/app 本線は触らない）

目標: 承認済み `glam`（`bytemuck`+`libm`）と `wide` を `shared-sim` に入れ、`HotPhysicsPage` SoA 上の **慣性予測ホットパス**を Safe SIMD 既定に差し替える。**スカラー参照実装は残し**、同一入力でビット一致を単体テストする。

根拠: [mvcc_xpbd.md](../mvcc_xpbd.md) Q5 · [improvement-plan-2026-11-12.md](../../0_doc/evaluation/improvement-plan-2026-11-12.md)

---

## 保証する / しない（本スライス）

| 保証する | 保証しない |
|:---|:---|
| `glam` / `wide` が `alchemy-shared-sim` の依存として解決される（Q5） | host/app 本線配線・`ALCHEMY_SIM_BACKEND` |
| [`step_inertia`](../../client/shared-sim/src/mvcc/inertia.rs) が SIMD 既定の **SoA ビルディングブロック**（アダプタ未配線）；[`step_inertia_scalar`](../../client/shared-sim/src/mvcc/inertia.rs) が参照として残る | 拘束ソルバー全体の SIMD 化・`PageXpbdAdapter` / `predict_integrate` 置換（本番ティックは従来どおり Epoch XPBD） |
| 同一 `curr`/`prev`/重力/`dt` で scalar と SIMD の SoA 位置が **ビット一致** | クロスアーキ決定論（Q2 未決） |
| `math_det`（`-0.0` 正規化・有限検疫・IEEE `f32` √ による normalize）が `forbid(unsafe_code)` 下で動く | `-fma` ワークスペース一括 rustflags；√ の `libm` 経由は主張しない |
| 既存 `cargo test -p alchemy-shared-sim --lib mvcc` が Green のまま | Phase 2 wgpu / Phase 3 dirty デルタ |

---

## Phase

| Phase | 内容 | 状態 |
|:---|:---|:---|
| 0 | Slice + Q5 文書反映 | **done** |
| 1 | workspace / shared-sim に `glam`・`wide` 追加 | **done** |
| 2 | `math_det` + `inertia`（scalar 参照 + SIMD 既定）+ 一致テスト | **done** |

---

## 完了条件

1. `cargo test -p alchemy-shared-sim --lib mvcc` Green（inertia scalar≡simd 含む）。
2. Epoch / host / app 本線に差分なし（アダプタの XPBD 経路は置換しない）。
3. roadmap 目次で本スライスが `done`。
