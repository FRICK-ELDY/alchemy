# Cloudflare Tunnel（x001a / x001e）

拠点の **auth / world / assets**（`127.0.0.1:4002` / `4003` / `4004`）を、ルーターのポート開放なしで HTTPS 公開する。

**Windows 初回〜常用の手順書:** [`.workspace/0_doc/manual/cloudflare-tunnel-windows.md`](../../.workspace/0_doc/manual/cloudflare-tunnel-windows.md)  
正本 Slice: [`.workspace/roadmap/slices/x001a-https-edge-tunnel.md`](../../.workspace/roadmap/slices/x001a-https-edge-tunnel.md) · [x001e](../../.workspace/roadmap/slices/x001e-p0-trust-boundary.md)

## 保証する / しない

| 保証する | 保証しない |
|:---|:---|
| Named Tunnel + ホスト名振り分けのテンプレ | Quick Tunnel・path 振り分け全般 |
| 公網ポート開放ゼロで 3 サービスへ HTTPS 到達 | Zenoh WAN（→ x001b） |
| credentials / 実 `config.yml` をリポに入れない | DNS / Zone の完全自動化 |
| **公開 origin は release / `MIX_ENV=prod` のみ**（x001e） | cloudflared HA・複数拠点 |
| auth `/dev/*` は ingress で 404（mailbox 二重封鎖） | Cloudflare Access のフル構成 |

## 前提（重要・x001e）

- Cloudflare アカウントと **Zone**（カスタムドメイン。DNS 手作業可）
- 拠点の auth / world / assets は **`docker-compose.prod.yml`（release）** で起動済み
  - **禁止:** `docker compose up`（各サーバの `target: dev` / `MIX_ENV=dev`）を Tunnel の後ろに置くこと
  - dev の `/dev/mailbox` は認証なしで見えるため、forgot-password → mailbox → reset でアカウントを奪える
- [`cloudflared`](https://developers.cloudflare.com/cloudflare-one/connections/connect-apps/install-and-setup/installation/) が PATH にある
- **WSL:** cloudflared は `127.0.0.1:4002–4004` に届く側（WSL 内推奨）で起動する

`AUTH_JWKS_URL`（world / assets → auth）は Docker 内のまま。Tunnel は外向け入口だけ。

## セットアップ

### 1. Tunnel 作成

```bash
cloudflared tunnel login
cloudflared tunnel create alchemy-home
```

表示された **Tunnel ID** と `~/.cloudflared/<TUNNEL_ID>.json` を控える。

### 2. credentials 配置

```bash
cp ~/.cloudflared/<TUNNEL_ID>.json edge/cloudflared/credentials.json
```

`credentials.json` は gitignore 済み。リポにコミットしない。

### 3. config.yml

```bash
cp edge/cloudflared/config.yml.example edge/cloudflared/config.yml
```

`config.yml` で次を置換する:

- `tunnel: <TUNNEL_ID>`
- `hostname:` の `example.tld` → 自分の Zone
- `credentials-file` は相対パス `./credentials.json` のまま（`cloudflared` 起動時の cwd に注意。下記「起動」参照）

任意: [`.env.example`](.env.example) を `.env` にコピーしてホスト名メモ用。

### 4. DNS（CNAME・Proxied）

Zone に次を追加（Dashboard または CLI）。ターゲットは `<TUNNEL_ID>.cfargotunnel.com`、**Proxy オン**。

| 名前 | 種別 | ターゲット |
|:---|:---|:---|
| `auth` | CNAME | `<TUNNEL_ID>.cfargotunnel.com` |
| `world` | CNAME | `<TUNNEL_ID>.cfargotunnel.com` |
| `assets` | CNAME | `<TUNNEL_ID>.cfargotunnel.com` |

CLI 例:

```bash
cloudflared tunnel route dns alchemy-home auth.example.tld
cloudflared tunnel route dns alchemy-home world.example.tld
cloudflared tunnel route dns alchemy-home assets.example.tld
```

### 5. 拠点サービス起動（**prod / release のみ**）

各サーバで `.env.prod.example` → `.env.prod` を用意し、release compose を使う:

```bash
# auth（JWT PEM を secrets/ に配置してから）
cd auth-server && docker compose -f docker-compose.prod.yml --env-file .env.prod up -d --build && cd ..
docker compose -f auth-server/docker-compose.prod.yml --env-file auth-server/.env.prod \
  exec auth bin/auth eval 'Auth.Release.migrate()'

cd world-server && docker compose -f docker-compose.prod.yml --env-file .env.prod up -d --build && cd ..
# Room Token 鍵は named volume `world_prod_keys`。`down -v` で消すと鍵が再生成され
# 既存 Room Token はすべて無効 → クライアントは再 login / 再発行が必要。

cd assets-server && docker compose -f docker-compose.prod.yml --env-file .env.prod up -d --build && cd ..
# migrate は entrypoint 経由（su-exec assets）。素の `exec` は使わない。
docker compose -f assets-server/docker-compose.prod.yml --env-file assets-server/.env.prod \
  --profile migrate run --rm assets-migrate
# または: bash .scripts/assets-prod-migrate.sh
```

ローカル開発用の `docker compose up`（dev）は **Tunnel に繋がない**。

### 6. Tunnel 起動

`credentials-file: ./credentials.json` は起動時の cwd 基準。このディレクトリから起動する:

```bash
cd edge/cloudflared && cloudflared tunnel --config config.yml run
```

## Health 確認

```bash
curl -sf https://auth.example.tld/health
curl -sf https://world.example.tld/health
curl -sf https://assets.example.tld/health
# mailbox は 404 であること
curl -o /dev/null -w "%{http_code}\n" https://auth.example.tld/dev/mailbox
```

縦通し（login → Room Token → assets）:

```bash
export AUTH_URL=https://auth.example.tld
export WORLD_URL=https://world.example.tld
export ASSETS_URL=https://assets.example.tld
bash .scripts/smoke-https-edge.sh
```

外クライアント env 例: [`../client.env.example`](../client.env.example)

## 注意

- ネイティブ client（ureq）なのでブラウザ CORS は対象外
- JWT / Room Token の `iss` / 鍵は変えない。変わるのは到達 URL のみ
- 拠点 host + 拠点 zenohd は従来どおり（本 Slice では Zenoh を変えない）
- release バイナリの host/client は `AUTH_USER` / `AUTH_PASSWORD`（または `ALCHEMY_AUTH_TOKEN`）必須。seed `alice`/`Password1` は debug のみ（x001e）
