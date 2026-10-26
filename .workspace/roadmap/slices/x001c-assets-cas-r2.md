# x001c — assets CAS → R2（S3 互換バックエンド）

> **ID:** `x001c` · **slug:** `assets-cas-r2` · **status:** `active` · **area:** `[assets, docs]` · **depends_on:** `[x0003, x001a]`

目標: assets-server の **不変バイナリ置き場**をローカルディスクだけでなく **S3 互換（Cloudflare R2）**に切替可能にする。メタ（BlobRef / Quota）・JWT 門番・既存 HTTP API は維持し、client の `ASSETS_URL` 契約は変えない（GET/PUT は引き続き assets-server 経由＝代理）。

正本: [assets-server.md](../../0_doc/architecture/assets-server.md) · [overview.md](../../0_doc/architecture/overview.md)（R2 CAS ⑥⑦）  
前提: [x0003](x0003-assets-cas-quota.md)（ローカル CAS）· [x001a](x001a-https-edge-tunnel.md)（外から assets HTTPS）

## 設計方針

| 層 | 役割 |
|:---|:---|
| assets-server | 権威: JWT・Quota・BlobRef。API パスは既存のまま |
| CAS backend | `local`（既定）または `s3`（R2 / 将来 NAS）。env で切替 |
| R2 | 不変バイナリのみ。キー = 任意 prefix + BLAKE3 hex |
| client | 変更なし（`GET/POST …/api/v1/blobs`） |

| Phase | 内容 | 状態 |
|:---|:---|:---|
| 0 | Slice 文書 + 目次 | done |
| 1 | CAS 抽象（Local / S3）+ runtime env | done |
| 2 | R2 runbook（バケット・API トークン・env）+ `.env.example` | done |
| 3 | smoke（local 回帰 + R2 任意）+ architecture / README | done |

## 保証する / しない

| 保証する | 保証しない |
|:---|:---|
| `ASSETS_CAS_BACKEND=local\|s3` で置き場切替 | client の CDN 直結・署名 URL 必須化 |
| S3 互換 PUT/GET/HEAD（R2 で検証） | Spatial VCS / Origin Cert / pgvector |
| 既存 API・Quota・dedupe 意味の維持 | 課金連動 Quota・マルチバケット |
| 秘密は env / gitignore（リポにキーを入れない） | R2 公開バケット・カスタムドメイン本配信 |
| local 既定で CI / Docker が従来どおり通る | 既存 local CAS からの自動マイグレーション |

## やらない（後続）

- 署名付き URL / 302 で client → R2 直 DL（Egress 0 を最大化する縦）
- Vault / Folder / Commit
- QuObjects（NAS）本番運用（S3 互換なら同一 backend で後から可）
- エッジ JWT / Zenoh ACL

## 成功条件（デモ）

1. `BACKEND=local` で従来 smoke（`smoke-assets.sh`）が通る
2. R2 バケット + API トークンを用意し `BACKEND=s3` で upload → download → bytes match
3. Tunnel（x001a）経由でも同じ API で取得できる（任意）

## 環境変数（案）

| 変数 | 既定 | 内容 |
|:---|:---|:---|
| `ASSETS_CAS_BACKEND` | `local` | `local` / `s3` |
| `ASSETS_CAS_PATH` | `priv/cas` | local 時のルート |
| `ASSETS_S3_BUCKET` | — | s3 時必須 |
| `ASSETS_S3_ENDPOINT` | — | `https://<ACCOUNT_ID>.r2.cloudflarestorage.com` |
| `ASSETS_S3_ACCESS_KEY_ID` | — | R2 API トークン |
| `ASSETS_S3_SECRET_ACCESS_KEY` | — | 同上 |
| `ASSETS_S3_REGION` | `auto` | R2 は `auto` |
| `ASSETS_S3_PREFIX` | `cas/` | オブジェクトキー接頭辞 |

## 起動

```bash
# local（従来）
cd assets-server && docker compose up -d
make smoke-assets

# R2（assets-server/.env で BACKEND=s3 など → docker compose up -d）
make smoke-assets-r2
```

## 実装メモ（方針）

- `Assets.CAS` は facade。Local は現行ディスク、S3 は `ex_aws` + `ex_aws_s3`
- `put_if_absent`: HEAD（または同等）で存在確認 → 無ければ PUT。CAS なので競合時は同一バイト前提
- ダウンロードは当面 **assets-server が R2 から読んで返す**（API 契約固定）。直結は次 Slice
- Docker 既定は `local` のまま。R2 は compose override / 手元 env

← [目次](../README.md)
