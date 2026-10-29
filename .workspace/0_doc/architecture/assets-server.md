# assets-server

> **正本コード**: [`assets-server/`](../../../assets-server/)  
> **ステータス**: x0003（ローカル CAS + Quota）+ **x001c**（S3/R2 バックエンド）+ **x001d**（s3 時 GET は署名 URL 302）  
> **目標たたき台**: [assets-server-draft.md](assets-server-draft.md)（BlobGrant / 5 層。実装正本ではない）  
> **脅威（手口×対策）**: [assets-threat-model.md](assets-threat-model.md)

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
| auth `sub` を **PUT・Quota の所有者キー**として使う | GET で `BlobRef.owner_sub` と照合すること |
| GET は **認証済み読み取り capability**（下表） | 非公開・有料アセットの confidentiality（→ BlobGrant） |

## GET capability モデル（現行・コード一致）

`GET /api/v1/blobs/:blake3` が保証する門番は次のとおり。**所有者 grant ではない。**

| 検査 | 現行 | 備考 |
|:---|:---|:---|
| Bearer JWT（auth JWKS） | **必須** | `AuthenticateUser` → `auth_sub`。未認証は 401 |
| blake3 が `blob_refs` に存在 | **必須** | なければ 404 |
| `auth_sub == BlobRef.owner_sub` | **しない** | `show` は `Blobs.fetch(blake3)` のみ。hash を知る認証済み主体は読める |
| License / 購入証明 | **しない** | [assets-server-draft.md](assets-server-draft.md) の BlobGrant 後続 |
| 公開バケット直リンク | **しない** | s3 でも API 経由の短命 presigned GET（302） |

意図する運用前提: Manifest 等で blake3 を渡された **入室済み・認証済み client** がメッシュを取る経路。hash 秘匿や owner 限定配信は現行 API の保証外（[trust-boundary.md](trust-boundary.md)）。所有権ロンダリング・Quota TOCTOU 等は [assets-threat-model.md](assets-threat-model.md)。

PUT（`POST /api/v1/blobs`）は `auth_sub` で Quota 消費・`BlobRef.owner_sub` 記録。同一 BLAKE3 の再アップロードは CAS 再書込なし・Quota 非加算（**owner_sub は付け替えない**）。

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
| POST | `/api/v1/blobs` | Bearer | raw body アップロード（Quota・owner_sub） |
| GET | `/api/v1/blobs/:blake3` | Bearer | **認証済み読み取り capability**。local: 200 代理。s3: **302** → R2 presigned GET。owner 照合なし |
| GET | `/api/v1/quota` | Bearer | `{used_bytes, limit_bytes}`（自 `auth_sub`） |

## 最小オブジェクトモデル

| エンティティ | 役割 |
|:---|:---|
| BlobRef | `blake3` + `size` + `mime` + `owner_sub`（**Quota / 初回 PUT 帰属**。GET 認可キーではない） |
| QuotaLedger | `owner_sub` + `used_bytes` + `limit_bytes` |

```mermaid
flowchart LR
  UserRef[auth_sub] --> QuotaLedger
  UserRef -->|"PUT only"| BlobRef
  BlobRef -.->|"blake3 key"| CASLocal[(Local_CAS)]
  BlobRef -.->|"blake3 key"| CASR2[(R2_S3)]
  Client -->|"GET Bearer capability"| API[assets_HTTP]
  API -->|"exists? local 200"| CASLocal
  API -->|"exists? s3 302"| Client
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

- BlobGrant / CasObject 分離（[assets-server-draft.md](assets-server-draft.md)）— 非公開・購入品の GET 認可
- [assets-threat-model.md](assets-threat-model.md) 目標列（PoP・`reserved_bytes`・Immutable Tag・配信ドメイン分離・Safe HTTP・jti 等）
- カスタムドメイン / CDN キャッシュ本配信
- 署名付き Upload（client → R2 直 PUT）— Content-Length 固定署名を必須に
- Vault / Folder / Commit
- Ash、pgvector、Origin Cert
