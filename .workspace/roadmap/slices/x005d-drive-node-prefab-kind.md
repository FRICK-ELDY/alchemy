# x005d — DriveNode kind=prefab

> **ID:** `x005d` · **slug:** `drive-node-prefab-kind` · **status:** `planned` · **area:** `[assets]` · **depends_on:** `[x005c]`  
> **ladder:** Wave 2 · [inventory-ladder.md](../inventory-ladder.md)

目標: DriveNode に **`kind=prefab`** を追加する。ペイロード中身の正本スキーマは [x005e](x005e-prefab-payload-schema.md)。本 Slice は **ノード種別と参照の器**まで。

## 保証する / しない

| 保証する | 保証しない |
|:---|:---|
| `kind=prefab` を作成・一覧・削除できる | Prefab JSON の意味論検証の完全実装（x005e） |
| prefab ノードが payload blake3（grant_required）を指せる | take-out / put-back（x005f / x0060） |
| folder 配下に prefab を置ける | Vcs 昇格（x0062） |
| 未知 kind は fail-closed | Shop / Tag ピン |

## 設計メモ

- opaque 単一「何でも入り blob」を prefab の唯一表現にしない（x005e で Tree）
- `vcs_repo_id` はまだ null 固定でよい

| Phase | 内容 | 状態 |
|:---|:---|:---|
| 0 | 本 Slice | planned |
| 1 | kind 拡張 + API | planned |
| 2 | テスト + draft 注記 | planned |

## 完了条件

1. prefab ノード CRUD が folder/blob と共存
2. 既存 folder/blob テスト緑

## やらない

- host spawn
- Commit / Branch

← [目次](../README.md) · [ladder](../inventory-ladder.md)