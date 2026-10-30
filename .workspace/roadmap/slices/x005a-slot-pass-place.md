# x005a — 渡す・置く（Slot 操作）

> **ID:** `x005a` · **slug:** `slot-pass-place` · **status:** `planned` · **area:** `[host, sim]` · **depends_on:** `[x0057, x0059]`  
> **ladder:** Wave 2 · [inventory-ladder.md](../inventory-ladder.md)

目標: 空間内で **渡す**（他 user_root の held へ）と **置く**（parent 解除＝世界）を host が保証する。

## 保証する / しない

| 保証する | 保証しない |
|:---|:---|
| pass: 保持者が他 root の held に移せる（距離／権限の最小ゲート） | Steal（同意なし奪取） |
| place: held → parent None + 物理 Dynamic 復帰（既存 release と整合） | 回転追従・投擲 impulse |
| 同一セッション内で親子と Grab 状態が矛盾しない | 永続インベントリ |
| Intent／操作の入口が文書化される | undo（→ [x005b](x005b-slot-session-undo.md)。Resonite 相当の手元 A↔B） |

## 設計メモ

- 権限ゲートは「渡す側が現 held 保持者」程度の最小。role capability 深掘りは別
- 遷移の履歴化・手元 undo/redo は [x005b](x005b-slot-session-undo.md)（Resonite: A→B のあと undo で B→A）

| Phase | 内容 | 状態 |
|:---|:---|:---|
| 0 | 本 Slice | planned |
| 1 | pass / place API + host 配線 | planned |
| 2 | テスト + client.md 注記 | planned |

## 完了条件

1. 2 slot 間 pass の単体またはホストテスト
2. place 後 Grab なし・Dynamic

## やらない

- Drive / Prefab 保存
- CRDT

← [目次](../README.md) · [ladder](../inventory-ladder.md)