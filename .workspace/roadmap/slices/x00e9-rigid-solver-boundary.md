# x00e9 — RigidSolver 境界のみ（Phase 3 枠）

> **ID:** `x00e9` · **slug:** `rigid-solver-boundary` · **status:** `done` · **area:** `[sim]` · **depends_on:** `[x00e8]`  
> **優先度:** 品質本線 **Phase 3**（枠のみ・実装は別決）  
> **注意:** **rapier3d / AVBD は未採択**（guardrails: 新規依存は人承認）

目標: `RigidSolver`（または同等）の **境界インターフェースだけ**を置き、Ring 2 Step 2 への差し替え口を文書・型で固定する。本スライスで本格剛体ソルバ実装や rapier 再導入はしない。

根拠: [scene_store_sim_arena.md](../scene_store_sim_arena.md) §1.4 / §7 Phase 3 / guardrails

---

## 保証する / しない（本スライス）

| 保証する | 保証しない |
|:---|:---|
| RigidSolver 境界（trait / モジュール口）と「実装は別決」の明示 | rapier3d / AVBD の再導入・実装 |
| 安定契約: Work/sheet 等は `Self` 注入、`step(dt)` のみ（呼び出し時面参照へ黙って広げない） | 規模本線 CCU KPI |
| 現行 Native XPBD／不等式接触を無断削除しない | |

## 触ったファイル

- `client/shared-sim/src/rigid_solver/mod.rs` — 境界 docs + Noop / dyn / bit テスト
- `client/shared-sim/src/rigid_solver/solver.rs` — `RigidSolver` + `NoopRigidSolver` + `COMP_RIGID_BODY`
- `client/shared-sim/src/lib.rs` — `pub mod rigid_solver`；crate docs
- `client/shared-sim/src/avatar_rig/mod.rs` — Non-guarantees を境界 vs 実装で整合
- [scene_store_sim_arena.md](../scene_store_sim_arena.md) / [README.md](../README.md) — status・次着手

## 完了条件

1. 境界型／モジュールが shared-sim にあり、呼び出し側は未配線または Noop 可。
2. 新規物理クレートを人承認なしで追加しない。
