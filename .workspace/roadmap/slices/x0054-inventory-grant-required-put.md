# x0054 — inventory PUT は常に grant_required

> **ID:** `x0054` · **slug:** `inventory-grant-required-put` · **status:** `planned` · **area:** `[assets]` · **depends_on:** `[x004f]`  
> **ladder:** Wave 0 · [inventory-ladder.md](../inventory-ladder.md)

目標: 個人インベントリ用バイナリの PUT 経路を **常に `access=grant_required`** に固定する。部屋メッシュ用 `POST /api/v1/blobs`（既定 `authenticated`）は壊さない。

## 保証する / しない

| 保証する | 保証しない |
|:---|:---|
| inventory 専用 PUT（例: `POST /api/v1/inventory/blobs` または同等）は常に `grant_required` | GET 既定を常時 Grant 必須にする案 B 本体 |
| finalize で uploader に `:owned` BlobGrant | room join 連動読取 Grant |
| owner GET 200 / 他 user GET 403 | DriveNode / Prefab / Vault 表 |
| 既存部屋メッシュ DL（`authenticated`）回帰緑 | client UI・host spawn |

## 設計メモ

- 既存 `x-alchemy-blob-access: grant_required` でも達成可能だが、**経路を分ける**ことで「インベントリは私有」を API 契約として固定する
- 案 B（部屋メッシュ含む全 GET＝Grant）は別 Slice（本 Slice 外）

| Phase | 内容 | 状態 |
|:---|:---|:---|
| 0 | 本 Slice + 目次 + ladder | planned |
| 1 | upload 固定 + HTTP + テスト | planned |
| 2 | [assets-server.md](../../0_doc/architecture/assets-server.md) 保証表 | planned |

## 完了条件

1. inventory PUT → `access=grant_required` + `:owned` Grant
2. 他 user GET 403 / owner 200
3. ヘッダ無し部屋メッシュ経路の smoke / テスト緑

## やらない

- StorageVault / Drive / Prefab
- GET 常時 Grant（案 B）
- 新規 Hex 依存

← [目次](../README.md) · [ladder](../inventory-ladder.md)