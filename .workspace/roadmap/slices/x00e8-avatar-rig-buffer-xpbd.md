# x00e8 — AvatarRigBuffer 揺れもの XPBD（Phase 2）

> **ID:** `x00e8` · **slug:** `avatar-rig-buffer-xpbd` · **status:** `planned` · **area:** `[sim, client]` · **depends_on:** `[x00e1]`  
> **優先度:** 品質本線 **Phase 2**（1a–1c の後）  
> **後続:** [x00e9](x00e9-rigid-solver-boundary.md)

目標: アバター末端ボーン・揺れものを空間 `Slot` に載せず、ローカル連続配列 **`AvatarRigBuffer`** 上の XPBD として分離する。空間 Slot 同期は Root / Head / Hands（＋フルトラ数点）に限定する布石を固める。

根拠: [scene_store_sim_arena.md](../scene_store_sim_arena.md) §4.3 / §7 Phase 2

---

## 保証する / しない（本スライス）

| 保証する | 保証しない |
|:---|:---|
| `AvatarRigBuffer` と揺れもの XPBD の最小経路；Slot 非管理 | 空間 Slot へのボーン全登録 |
| Phase 1 の scene_store / sim_arena 契約を壊さない | rapier3d、規模 KPI 宣言 |

## 完了条件

1. 揺れものが Scene Store Slot 爆発を起こさない境界がコードとテストで明示される。
2. 新規依存は人承認なしで追加しない。
