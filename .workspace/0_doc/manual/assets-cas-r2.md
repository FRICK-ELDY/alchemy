# assets CAS → Cloudflare R2（x001c）

assets-server の不変バイナリ置き場を **ローカルディスク**から **R2（S3 互換）**へ切替える手順。  
メタ・Quota・JWT・HTTP API は従来どおり。client の `ASSETS_URL` は変更不要（代理 GET/PUT）。

正本 Slice: [x001c-assets-cas-r2.md](../../roadmap/slices/x001c-assets-cas-r2.md)  
実装: [`assets-server/`](../../../assets-server/) · env 例: [`assets-server/.env.example`](../../../assets-server/.env.example)

---

## 1. Cloudflare 側

1. Dashboard → **R2** → バケット作成（例: `alchemy-cas`）
2. **Manage API Tokens** → Object Read & Write（当該バケット限定で可）
3. 控えるもの:
   - Access Key ID / Secret Access Key
   - S3 API エンドポイント: `https://<ACCOUNT_ID>.r2.cloudflarestorage.com`

公開バケットやカスタムドメインは本 Slice では不要（直結 DL は後続）。

---

## 2. assets-server 環境変数

| 変数 | 例 |
|:---|:---|
| `ASSETS_CAS_BACKEND` | `s3` |
| `ASSETS_S3_BUCKET` | `alchemy-cas` |
| `ASSETS_S3_ENDPOINT` | `https://<ACCOUNT_ID>.r2.cloudflarestorage.com` |
| `ASSETS_S3_ACCESS_KEY_ID` | （トークン） |
| `ASSETS_S3_SECRET_ACCESS_KEY` | （トークン） |
| `ASSETS_S3_REGION` | `auto`（既定） |
| `ASSETS_S3_PREFIX` | `cas/`（既定） |

Docker（おすすめ）: `assets-server/.env` に上表を書き（[`.env.example`](../../../assets-server/.env.example) 参照）、同ディレクトリで:

```powershell
cd D:\github\frick\alchemy\assets-server
docker compose up -d
```

`docker-compose.yml` は `.env` を読み取りコンテナへ渡す。未設定時の既定は **local**。

---

## 3. 確認

auth が起動している前提:

```bash
# local 回帰
make smoke-assets

# R2（assets-server を ASSETS_CAS_BACKEND=s3 で起動済み）
make smoke-assets-r2
```

（macOS / Linux / WSL。ネイティブ Windows の `bash` / `make` 不足時は WSL を使う。）
成功: upload → download の bytes match。R2 ダッシュボードに `cas/<blake3>` が見える。

Tunnel（x001a）併用時は `ASSETS_URL=https://assets.…` のまま同じ API。

---

## 保証する / しない

| 保証する | 保証しない |
|:---|:---|
| env 1 系統で local ↔ R2 切替 | 既存 local CAS の自動移行 |
| 秘密をリポに入れない | 署名 URL・CDN 直結 |
| local 既定で CI が通る | QuObjects 本番 runbook（同一 S3 backend で可） |
