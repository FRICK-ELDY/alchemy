# x0062 — Prefab → VcsRepo 昇格

> **ID:** `x0062` · **slug:** `prefab-promote-vcs` · **status:** `planned` · **area:** `[assets]` · **depends_on:** `[x0061]`  
> **ladder:** Wave 4 · [inventory-ladder.md](../inventory-ladder.md)

目標: 既存 Prefab DriveNode を **`VcsRepo` に昇格**し、初回 Commit の Tree を現行 Prefab ペイロードと同一論理にする（C+3 入口）。

## 保証する / しない

| 保証する | 保証しない |
|:---|:---|
| `kind=prefab` → `vcs_repo`（または `vcs_repo_id` 付与）が同一 owner TX で完了 | Branch/Tag UI（→ [x0064](x0064-vcs-branch-tag-minimal.md)） |
| 初回 Commit の Tree ≡ Prefab 子参照 | 3D Diff / Merge |
| 旧 blake3 Prefab パスが当面残る／読める方針 | Origin Cert（→ [x0065](x0065-origin-cert-thin.md)） |
| host 契約を壊さない（ItemRef 拡張は [x0063](x0063-item-ref-repo-rev.md)） | Shopfront Tag ピン |

## 設計メモ

- B+2 の同一性（node_id）を維持したまま昇格する。付け替えで node_id を捨てない

| Phase | 内容 | 状態 |
|:---|:---|:---|
| 0 | 本 Slice | planned |
| 1 | VcsRepo / Commit / Tree 表 + promote | planned |
| 2 | テスト + draft 正本化メモ | planned |

## 完了条件

1. promote 後に Repo と初回 Commit が存在する
2. Prefab 論理 Tree と Commit Tree が一致
3. 未 promote Prefab の take-out が引き続き動く

## やらない

- market / License Token
- verifier 本実装

← [目次](../README.md) · [ladder](../inventory-ladder.md)