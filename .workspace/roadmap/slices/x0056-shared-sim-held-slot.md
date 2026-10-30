# x0056 — shared-sim held Slot（root → held）

> **ID:** `x0056` · **slug:** `shared-sim-held-slot` · **status:** `planned` · **area:** `[sim]` · **depends_on:** `[x0030]`  
> **ladder:** Wave 1 · [inventory-ladder.md](../inventory-ladder.md)

目標: `alchemy-shared-sim` に **実行時親子**の最小 API を置く。最低保証は `user_root → held_entity`。

## 保証する / しない

| 保証する | 保証しない |
|:---|:---|
| SlotGraph（または同等）で child→parent を表せる | 任意深さの一般グラフ（→ [x0059](x0059-slot-graph-general.md)） |
| attach_held / clear_held が決定的 | Rapier kinematic・Intent・Zenoh |
| 単体テスト（親子の設定／解除） | pass / place / undo |
| host・client が同じ型を参照できる配置 | host への配線（→ [x0057](x0057-host-grab-held-slot.md)） |

## 設計メモ

- 永続 ID・blake3・Drive を知らない（層間インターフェース）
- SoA 寄り（parent 配列 / map）を優先。ホットパスで毎 tick の HashMap 新規確保を避ける

| Phase | 内容 | 状態 |
|:---|:---|:---|
| 0 | 本 Slice | planned |
| 1 | SlotGraph held API + tests | planned |
| 2 | shared-sim / client.md 注記 | planned |

## 完了条件

1. held 親子の設定・解除テスト緑
2. Rapier / assets 依存なし

## やらない

- host `apply_grab` 変更（次 Slice）
- 永続化

← [目次](../README.md) · [ladder](../inventory-ladder.md)