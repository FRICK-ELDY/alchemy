# x00e6 — PendingLocalOps ＋ Error Smoothing（app）

> **ID:** `x00e6` · **slug:** `pending-local-ops` · **status:** `planned` · **area:** `[client]` · **depends_on:** `[x00dc, x00e1]`  
> **優先度:** 品質本線 Phase 1c / app  
> **後続:** Phase 2 [x00e8](x00e8-avatar-rig-buffer-xpbd.md)

目標: client/app に **`PendingLocalOps` + Error Smoothing** を載せ、楽観予測を Published 面を汚さずに行う。TELEPORT 時は即スナップ（既存契約）。

根拠: [scene_store_sim_arena.md](../scene_store_sim_arena.md) §3.4 / Ring3 GPU / §7 Phase 1c

---

## 保証する / しない（本スライス）

| 保証する | 保証しない |
|:---|:---|
| PendingLocalOps と Error Smoothing；Published 面非汚染 | host 権威ループの変更必須化 |
| TELEPORT_THIS_TICK 時の補間スキップ整合 | SoftLease / TRASH 必須 |

## 完了条件

1. 予測誤差が Published Arc を書き換えないこと。
2. 補間ワープ防止（teleport）と両立する最小テストまたは手動手順。
