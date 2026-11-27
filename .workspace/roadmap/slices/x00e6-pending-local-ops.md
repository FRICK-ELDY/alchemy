# x00e6 — PendingLocalOps ＋ Error Smoothing（app）

> **ID:** `x00e6` · **slug:** `pending-local-ops` · **status:** `done` · **area:** `[client]` · **depends_on:** `[x00dc, x00e1]`  
> **優先度:** 品質本線 Phase 1c / app  
> **後続:** Phase 2 [x00e8](x00e8-avatar-rig-buffer-xpbd.md)（改称は [x00e7](x00e7-mvcc-rename-sim-pages.md)）

目標: client/app に **`PendingLocalOps` + Error Smoothing** を載せ、楽観予測を Published 面を汚さずに行う。TELEPORT 時は即スナップ（既存契約）。

根拠: [scene_store_sim_arena.md](../scene_store_sim_arena.md) §3.4 / Ring3 GPU / §7 Phase 1c

---

## 保証する / しない（本スライス）

| 保証する | 保証しない |
|:---|:---|
| PendingLocalOps と Error Smoothing；Published 面非汚染 | host 権威ループの変更必須化 |
| TELEPORT_THIS_TICK 時の補間スキップ整合 | SoftLease / TRASH 必須 |
| Grab 非依存の一般器（Pose；将来 Property 等） | レーザー／物理ボタン本配線；Grab `owned_prediction` 置換；Arena→GPU TELEPORT 配線 |

## 触ったファイル

- `client/app/src/pending_local_ops.rs` — `PendingLocalOps` / `ErrorSmoothing` / `PendingPose` / `ReconcileMode` / `PendingKind::Pose`；`PendingLocalGate`（Mutex + Release/Acquire `any`）；Published 非汚染・teleport Snap・ゲート同期テスト
- `client/app/src/main.rs` — `mod` + `SharedState.pending_local` 初期化
- `client/app/src/scene.rs` — `pending_local: PendingLocalGate`（mutation API は Gate のみ）；`display_pose_for_draw` 第2優先
- `client/app/src/owned_prediction.rs` — `PredictionPose` / `ReconcileMode` / 閾値を `pending_local_ops` から type alias / re-export（Grab セマンティクスは残置）

## 完了条件

1. 予測誤差が Published Arc を書き換えないこと。 — `smoothing_does_not_mutate_published_arc`
2. 補間ワープ防止（teleport）と両立する最小テストまたは手動手順。 — `teleport_skips_blend_and_snaps`
3. ゲート: push→`any` true / Snap・clear→false — `gate_release_acquire_push_snap_clear`
