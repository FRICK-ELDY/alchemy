# x004f — BlobGrant 最小（案 A＝移行、目標は案 B）

> **ID:** `x004f` · **slug:** `blob-grant-mvp` · **status:** `done` · **area:** `[assets, docs]` · **depends_on:** `[x003f]`  
> **優先度:** **P4-5**（MVP / 案 A）

目標: `BlobGrant` 台帳と `CasObject.access`（既定 `:authenticated` / 任意 `:grant_required`）を入れる。部屋メッシュ DL（Manifest → 自分の JWT → GET）は壊さない。**本質・目標形は案 B**（GET の SSoT = 常に Grant）。本スライスは移行段。

根拠: [improvement-plan-2026-10-28.md](../../0_doc/evaluation/improvement-plan-2026-10-28.md) **P4-5** · [assets-server-draft.md](../../0_doc/architecture/assets-server-draft.md) · 正本 [assets-server.md](../../0_doc/architecture/assets-server.md)

## 方針

| | 内容 |
|:---|:---|
| 目標（案 B） | GET 可否の SSoT は常に `BlobGrant`。部屋メッシュも入室連動の読取 Grant（または短命トークン）が先 |
| 本スライス（案 A） | 台帳（`:owned`）+ `grant_required` の閉じ方だけ。既定 GET は現行 capability |
| 後続 | room join 連動 Grant → 既定を閉じる（別スライス。本スライスではやらない） |

## 保証する / しない

| 保証する | 保証しない |
|:---|:---|
| PUT finalize で uploader に `:owned` Grant が立つ | Vault / purchased / mail_temp / 出品ロック |
| `access=authenticated`（既定）は任意認証済みが読める | 部屋配布済み hash の秘密性（threat-model ⑥） |
| `access=grant_required` は Grant 保有者のみ GET（他人 403） | room スコープ grant・Manifest 連動付与（→ 案 B） |
| 未知 `access` は fail-closed（403）。要求 access 不一致は 409 | 存在秘匿（403 vs 404 オラクルは意図） |
| BlobRef は引き続き Quota 単位 | BlobRef → Grant 表統合・GET 既定を Grant 必須へ切替 |
| 既存 smoke / 入室 mesh DL はヘッダ無しで緑 | market Webhook・License Token・門番なし download API |

## 設計

1. **`cas_objects.access`**: `"authenticated"` | `"grant_required"`（既定 authenticated。先勝ち）
2. **`blob_grants`**: UNIQUE(`grantee_sub`, `blake3`) + `grant_type`（MVP は `owned` のみ付与）
3. **PUT**: ヘッダ `x-alchemy-blob-access: grant_required`（未指定=authenticated、未知値=400）
4. **GET**: `grant_required` のときだけ `auth_sub` の Grant 必須。無ければ 403
5. **Backfill**: 既存 `blob_refs` → `owned` grants

| Phase | 内容 | 状態 |
|:---|:---|:---|
| 0 | Slice + 目次 | done |
| 1 | Migration + schema | done |
| 2 | finalize Grant + fetch 門番 + controller | done |
| 3 | テスト | done |
| 4 | architecture / improvement-plan / draft 注記 | done |

## 完了条件

- 既定 PUT → 他 user GET 200（回帰）
- `grant_required` PUT → owner 200 / 他 user 403
- finalize 後に `blob_grants` に `:owned`
- `mix test`（assets）緑。保証表・コード・slice 一致

## やらない

- GET 常時 Grant 必須（案 B 本体）
- room join 連動読取 Grant / 短命トークン
- StorageVault / `:purchased` API / Drive / Comms
- 新規 Hex 依存・client 変更

← [目次](../README.md)
