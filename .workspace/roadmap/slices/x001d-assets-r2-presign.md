# x001d — R2 署名 URL 直結

> **ID:** `x001d` · **slug:** `assets-r2-presign` · **status:** `active` · **area:** `[assets, client, docs]` · **depends_on:** `[x001c]`

目標: `ASSETS_CAS_BACKEND=s3` のとき、`GET /api/v1/blobs/:blake3` を JWT 門番後に **短命署名 URL への 302** にし、client / smoke が **R2 から直取得**する（assets-server は本体を代理転送しない）。local 既定は従来どおり 200 代理。

正本: [assets-server.md](../../0_doc/architecture/assets-server.md) · [overview.md](../../0_doc/architecture/overview.md)（⑦ CDN/S3 直 DL）  
前提: [x001c](x001c-assets-cas-r2.md)（S3/R2 CAS バックエンド）

## 設計方針

| 層 | 役割 |
|:---|:---|
| assets-server | JWT・BlobRef 確認 → s3 時は presigned GET の `Location` を返す |
| R2 | 不変バイナリを署名 URL で配信（公開バケット不要） |
| client / smoke | 入口は従来 API。リダイレクト追従で最終ボディ取得 |
| local CAS | 変更なし（200 + 代理ボディ） |

| Phase | 内容 | 状態 |
|:---|:---|:---|
| 0 | Slice 文書 + 目次 | done |
| 1 | `CAS.S3.presign_get` + Blobs/Controller 302 + TTL env | done |
| 2 | client リダイレクト確認 + smoke `-L` + R2 確認 | done |
| 3 | architecture / manual / `.env.example` | done |

## 保証する / しない

| 保証する | 保証しない |
|:---|:---|
| s3 時 GET は短命署名 URL へ 302 | 公開バケット・CDN カスタムドメイン |
| JWT なしは 401 のまま | Upload の署名 URL 直 PUT |
| local 既定は代理 GET のまま | Vault / Spatial VCS |
| smoke / client がリダイレクト追従で通る | P2P キャッシュ・課金 Quota |

## やらない（後続）

- カスタムドメイン / CDN キャッシュ本配信
- 署名付き Upload（client → R2 直 PUT）
- Vault / Folder / Commit

## 成功条件（デモ）

1. `BACKEND=local` で `make smoke-assets` が通る（代理 GET）
2. `BACKEND=s3` で upload → GET が **302** → `curl -L` / client で bytes match
3. s3 経路では assets-server がオブジェクト本体を `CAS.read` しない（presign のみ）

## 環境変数

| 変数 | 既定 | 内容 |
|:---|:---|:---|
| `ASSETS_S3_PRESIGN_TTL_SECONDS` | `300` | 署名 URL 有効秒（s3 時） |

（バケット・キー等は x001c と同じ。）

## 起動

```bash
# local（従来）
make smoke-assets

# R2 直結（assets-server を ASSETS_CAS_BACKEND=s3 で起動済み）
make smoke-assets-r2
```

← [目次](../README.md)
