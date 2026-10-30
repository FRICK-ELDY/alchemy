# x005c — DriveNode（folder / blob）

> **ID:** `x005c` · **slug:** `drive-node-folder-blob` · **status:** `planned` · **area:** `[assets]` · **depends_on:** `[x0054, x0058]`  
> **ladder:** Wave 2 · [inventory-ladder.md](../inventory-ladder.md)

目標: 個人 Drive の最小階層 — **folder** と **blob 葉**（blake3 参照）— を同一 TX 境界で持てる。

## 保証する / しない

| 保証する | 保証しない |
|:---|:---|
| `node_id` 安定・`owner_sub`・`parent_id`・`kind=folder\|blob` | `kind=prefab`（→ [x005d](x005d-drive-node-prefab-kind.md)） |
| blob 葉は既存 CasObject + Grant（inventory 私有）を参照 | Mail / Comms / Shortcut 本格 |
| 所有者のみ CRUD（Ash Policy または同等の最小門番） | 共有フォルダ・Group Vault |
| 循環 parent 拒否 | VcsRepo・Shop |

## 設計メモ

- 昇格用に後で `vcs_repo_id` nullable / `kind=vcs_repo` を足せる余白を schema コメントに残す
- Mail は本 Slice 外（インベントリ非ブロッカー）

| Phase | 内容 | 状態 |
|:---|:---|:---|
| 0 | 本 Slice | planned |
| 1 | Migration + schema + CRUD API | planned |
| 2 | テスト + assets-server.md / draft | planned |

## 完了条件

1. folder 作成・blob 葉紐付け・一覧が owner のみ可能
2. 他 user 403
3. 部屋メッシュ経路非破壊

## やらない

- Prefab / take-out
- CommsPost

← [目次](../README.md) · [ladder](../inventory-ladder.md)