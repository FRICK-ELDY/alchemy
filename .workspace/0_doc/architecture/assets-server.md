# assets-server

> **正本コード**: [`assets-server/`](../../../assets-server/)  
> **ステータス**: x0003（ローカル CAS + Quota）+ **x001c**（S3/R2 バックエンド）+ **x001d**（s3 時 GET は署名 URL 302）

## 図上の位置

[overview.md](overview.md) の⑤⑥⑦経路のサーバー側。バイナリ置き場は **local disk** または **S3 互換（R2）**（env 切替）。s3 時の GET は **presigned URL へ 302**（client → R2 直 DL）。

## 保証すること / しないこと

| 保証する | 保証しない |
|:---|:---|
| メタ（BlobRef / QuotaLedger）を SQLite に置く | GLB/KTX2 本体を DB に置く |
| 不変バイナリを CAS（BLAKE3 キー）に置く | Vault / Folder / Commit（Spatial VCS） |
| `ASSETS_CAS_BACKEND=local\|s3` で置き場切替（x001c）。**test では runtime が CAS env を無視**（x001e） | 公開バケット・CDN カスタムドメイン |
| s3 時 GET は短命署名 URL へ 302（x001d） | Upload の署名 URL 直 PUT |
| Quota 超過のアップロード拒否（既定 1GiB） | 課金連動 Quota / pgvector / Origin Cert / Ash 本格 |
| auth `sub` を所有者・Quota キーとして使う | 既存 local CAS の自動マイグレーション |

## 層の境界

| 層 | 置くもの | 置かないもの |
|:---|:---|:---|
| DB (Ecto SQLite) | owner_sub, blake3, size, mime, used_bytes | GLB/KTX2 本体 |
| CAS local | 不変バイナリ（BLAKE3 hex ファイル名） | ユーザー階層・権限 |
| CAS s3（R2） | 同上（キー = prefix + BLAKE3） | JWT / Quota |
| HTTP API | 門番・PUT 代理・GET（local=代理 / s3=302） | ストレージ固有の永続 URL 契約 |
| client | download（リダイレクト追従）+ BLAKE3 + wgpu | サーバー側メッシュ処理 / LOD |

## API

| Method | Path | Auth | 内容 |
|:---|:---|:---|:---|
| GET | `/health` | 不要 | 生存確認 |
| POST | `/api/v1/blobs` | Bearer | raw body アップロード |
| GET | `/api/v1/blobs/:blake3` | Bearer | local: 200 代理。s3: **302** → R2 presigned GET |
| GET | `/api/v1/quota` | Bearer | `{used_bytes, limit_bytes}` |

同一 BLAKE3 の再アップロードは CAS 再書込なし・Quota 非加算。

## 最小オブジェクトモデル

| エンティティ | 役割 |
|:---|:---|
| BlobRef | `blake3` + `size` + `mime` + `owner_sub` |
| QuotaLedger | `owner_sub` + `used_bytes` + `limit_bytes` |

```mermaid
flowchart LR
  UserRef[auth_sub] --> QuotaLedger
  UserRef --> BlobRef
  BlobRef -.->|"blake3 key"| CASLocal[(Local_CAS)]
  BlobRef -.->|"blake3 key"| CASR2[(R2_S3)]
  Client -->|"GET JWT"| API[assets_HTTP]
  API -->|"local 200"| CASLocal
  API -->|"s3 302"| Client
  Client -->|"presigned GET"| CASR2
```

## 起動

```bash
cd auth-server && docker compose up -d && cd ..
cd assets-server && docker compose up -d
bash .scripts/smoke-assets.sh
bash .scripts/smoke-assets-glb.sh   # Coin fixture → CAS
# R2 + 302: .workspace/0_doc/manual/assets-cas-r2.md · bash .scripts/smoke-assets-r2.sh
```

## 後続

- カスタムドメイン / CDN キャッシュ本配信
- 署名付き Upload（client → R2 直 PUT）
- Vault / Folder / Commit
- Ash、pgvector、Origin Cert
