# x0064 — Vcs Branch / Tag 最小

> **ID:** `x0064` · **slug:** `vcs-branch-tag-minimal` · **status:** `planned` · **area:** `[assets]` · **depends_on:** `[x0062]`  
> **ladder:** Wave 4 · [inventory-ladder.md](../inventory-ladder.md)

目標: `VcsRef` として **Branch と Tag の最小**（名前 → Commit）を保証する。Merge / 3D Diff はしない。

## 保証する / しない

| 保証する | 保証しない |
|:---|:---|
| Branch 先端の更新（fast-forward 最小） | 非 FF Merge・コンフリクト UI |
| Tag の不変ピン（作成後移動しない） | Shopfront の Tag ピン陳列（L5） |
| Ref 名の一意制約と不正名拒否 | Rollback UI の完成（API 最小は可） |
| ItemRef が Tag / Branch 名を rev に使えるなら文書化 | Origin Cert |

## 設計メモ

- Shop（L5）は Tag ピン前提 → 本 Slice 完了が Shop の先行条件
- Immutable Tag は脅威モデル目標列と整合（弱めない）

| Phase | 内容 | 状態 |
|:---|:---|:---|
| 0 | 本 Slice | planned |
| 1 | VcsRef schema + API | planned |
| 2 | テスト + draft | planned |

## 完了条件

1. Branch / Tag 作成・解決テスト緑
2. Tag 移動が拒否される（または仕様で禁止）

## やらない

- Shopfront
- 連合マーケット

← [目次](../README.md) · [ladder](../inventory-ladder.md)