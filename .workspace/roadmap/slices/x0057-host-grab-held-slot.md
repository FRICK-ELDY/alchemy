# x0057 — host Grab ↔ held Slot 同期

> **ID:** `x0057` · **slug:** `host-grab-held-slot` · **status:** `planned` · **area:** `[host, sim]` · **depends_on:** `[x0056, x0041]`  
> **ladder:** Wave 1 · [inventory-ladder.md](../inventory-ladder.md)

目標: 既存 Grab hold が立っている間、当該 entity の parent が **掴んでいる user_root** になるよう host が SlotGraph を同期する。

## 保証する / しない

| 保証する | 保証しない |
|:---|:---|
| Grab 開始で `held` parent = その slot の user_root | Steal・複数手・装着スロット一般化 |
| Grab 解放で held 親子解除（world 親＝無し） | pass / place 専用 API（→ [x005a](x005a-slot-pass-place.md)） |
| 既存 Grab 距離クランプ・finite 回帰 | Frame への parent フィールド追加 |
| 「掴んでいる」を SlotGraph から読める | 永続・Inventory UI |

## 設計メモ

- Rapier kinematic 追従は現状維持。SlotGraph は論理親子の SSoT
- Presence / Manifest ワイヤに parent を載せるかは本 Slice 外（必要なら別 Slice）

| Phase | 内容 | 状態 |
|:---|:---|:---|
| 0 | 本 Slice | planned |
| 1 | PhysicsWorld / ループから SlotGraph 同期 | planned |
| 2 | host 単体テスト | planned |

## 完了条件

1. Grab on → parent が user_root / Grab off → 解除
2. 既存 grab テスト緑

## やらない

- undo
- Drive / Prefab
- 本格 Walk 床物理の作り直し（別帯）

← [目次](../README.md) · [ladder](../inventory-ladder.md)