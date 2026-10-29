# x0041 — Grab 保持中 aim 距離クランプ（P1-6）

> **ID:** `x0041` · **slug:** `grab-hold-aim-clamp` · **status:** `done` · **area:** `[host, sim, docs]` · **depends_on:** `[x0021, x0030]`  
> **優先度:** **P1-6** — Grab 保持中の aim / offset に `MAX_GRAB_HIT_DISTANCE` を適用

目標: 正当な Grab 開始後に遠方 `aim` を流しても、箱を瞬間遠方へ移動させない。開始ゲートと同じ shared-sim 定数（5m）を、保持中の aim（user_root 中心）と hold offset 長に上限として適用する。

根拠: [improvement-plan-2026-10-28.md](../../0_doc/evaluation/improvement-plan-2026-10-28.md) **P1-6** · [opus-specific-proposals-2026-10-28.md](../../0_doc/evaluation/opus/opus-specific-proposals-2026-10-28.md)

## 保証する / しない

| 保証する | 保証しない |
|:---|:---|
| 保持中 `aim` は user_root から [`MAX_GRAB_HIT_DISTANCE`](../../../client/shared-sim/src/lib.rs) 以内に投影（f64 長・巨大 finite でも Inf にしない） | Steal / 回転追従 / throw |
| Grab 開始時 `offset = entity − aim` の長さも **同一定数**で上限（別名予算は持たない） | client pick 側の再クランプ（host が正） |
| 実体到達は最大でも概ね `2·MAX`（aim 球 + offset）。最終位置の `MAX` 再投影はしない | 最終 pose ≤ MAX への再投影 |
| 開始ゲート（player↔entity ≤ 5m）は従来どおり | Zenoh ACL・帯域 DoS |
| 超過は静かにクランプ（Grab は維持・drop しない） | クランプ後の見た目スムージング |

## 設計

1. **SSoT**: `alchemy_shared_sim::MAX_GRAB_HIT_DISTANCE`（開始・client pick・hold 共通）
2. **`finite::clamp_hold_aim(user_root, aim)`**: `(aim − root)` を球に投影して返す
3. **`finite::clamp_hold_offset`**: 開始時 offset 長を同球に投影
4. **`PhysicsWorld::apply_grab`**: 保持・開始の両方で aim を先にクランプ。開始時のみ offset もクランプしてから kinematic follow

| Phase | 内容 | 状態 |
|:---|:---|:---|
| 0 | Slice + 目次 | done |
| 1 | finite clamp + sim apply_grab | done |
| 2 | 単体テスト（遠方 aim / offset / f64） | done |
| 3 | architecture / improvement-plan | done |

## 完了条件

- 遠方 aim で箱が `2 · MAX`（aim+offset 上限）を超えて飛ばない
- `1e30` 級 aim でも有限座標に留まる
- `cargo test -p host-server` 緑。保証表・slice・コード一致
- P1-6 を improvement-plan 上 done

← [目次](../README.md)
