# x0059 — Slot グラフ一般化（任意 parent）

> **ID:** `x0059` · **slug:** `slot-graph-general` · **status:** `planned` · **area:** `[sim]` · **depends_on:** `[x0056]`  
> **ladder:** Wave 2 · [inventory-ladder.md](../inventory-ladder.md)

目標: held 専用を超え、entity の **任意 parent**（世界ルート＝parent 無し）を SlotGraph が表せるようにする。

## 保証する / しない

| 保証する | 保証しない |
|:---|:---|
| reparent(child, parent\|None) が循環を拒否する | pass / place のユーザー操作意味論（→ [x005a](x005a-slot-pass-place.md)） |
| 子孫列挙／直接子列挙 | CRDT・ネットワーク同期 |
| 単体テスト（循環・多重 reparent） | Rapier ジョイント自動生成 |
| held API が一般 API の特殊ケースとして残る／移行文書化 | 永続 Drive |

## 設計メモ

- 保証単位は **グラフ整合性**のみ。物理追従は host 側の別関心

| Phase | 内容 | 状態 |
|:---|:---|:---|
| 0 | 本 Slice | planned |
| 1 | 一般 reparent + 循環検査 + tests | planned |
| 2 | held API との関係を docs | planned |

## 完了条件

1. 循環 reparent が失敗する
2. held 既存テストが一般 API 上でも緑

## やらない

- undo スタック
- assets

← [目次](../README.md) · [ladder](../inventory-ladder.md)