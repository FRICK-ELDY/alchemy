# x0058 — StorageVault Personal 薄皮

> **ID:** `x0058` · **slug:** `vault-personal-peel` · **status:** `planned` · **area:** `[assets]` · **depends_on:** `[x003f, x004f]`  
> **ladder:** Wave 1 · [inventory-ladder.md](../inventory-ladder.md)

目標: `QuotaLedger` の上に **Personal StorageVault の読み取りビュー**を置き、後続 Drive／Grant が `vault` 語彙で話せるようにする。スキーマ大移行はしない。

## 保証する / しない

| 保証する | 保証しない |
|:---|:---|
| `Assets.Vault.personal/1`（または同等）が used/reserved/limit を返す | Group Vault |
| vault id の暫定安定キー（例: `owner_sub`）を文書化 | `UNIQUE(vault_id, blob_hash)` への BlobRef 移行 |
| 既存 Quota API 回帰 | 課金プラン連動・`:read_only` |
| architecture に「薄い皮」と明記 | DriveNode / Shop |

## 設計メモ

- Wave 1 は **表を増やさなくてよい**（ビュー／モジュールのみ可）
- 将来 UUID vault_id に替えるときは本 Slice の暫定キーを明示的に廃止する別 Slice

| Phase | 内容 | 状態 |
|:---|:---|:---|
| 0 | 本 Slice | planned |
| 1 | Vault モジュール + テスト | planned |
| 2 | assets-server.md / draft 注記 | planned |

## 完了条件

1. personal vault ビューが Quota と一致
2. 既存 quota / blob テスト緑

## やらない

- billing Webhook
- purchased / mail_temp grant_type

← [目次](../README.md) · [ladder](../inventory-ladder.md)