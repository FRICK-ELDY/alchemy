# x0060 — インベントリ戻す（Prefab 保存）

> **ID:** `x0060` · **slug:** `inventory-put-back` · **status:** `planned` · **area:** `[host, assets, client]` · **depends_on:** `[x005d, x005e, x005f]`  
> **ladder:** Wave 3 · [inventory-ladder.md](../inventory-ladder.md)

目標: 部屋内 subtree（held または選択範囲の最小）を **Prefab ペイロードとして保存**し DriveNode（kind=prefab）に戻す。

## 保証する / しない

| 保証する | 保証しない |
|:---|:---|
| 保存バイトは inventory PUT（grant_required）経由 | 空間内カメラの演出完成（フレーミング最小のみ可） |
| DriveNode prefab が新しい payload blake3 を指す（同一 TX で Grant 整合） | Vcs Commit（→ [x0062](x0062-prefab-promote-vcs.md)） |
| 保存後、元 entity を部屋から外す／残す方針が文書化され一貫 | 他ユーザ subtree の無断保存 |
| x005e スキーマ検証を通す | Mail 添付化 |

## 設計メモ

- 「逆保存」は overview Inventory タブの核。カメラは保存に必要な最小に限定

| Phase | 内容 | 状態 |
|:---|:---|:---|
| 0 | 本 Slice | planned |
| 1 | subtree → Prefab → Drive 更新 | planned |
| 2 | テスト + client.md Inventory 注記 | planned |

## 完了条件

1. put-back 後に Drive 上で prefab が読める
2. スキーマ不正は保存失敗
3. Quota / Grant 整合

## やらない

- rejoin E2E（x0061）
- Branch/Tag

← [目次](../README.md) · [ladder](../inventory-ladder.md)