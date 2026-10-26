# Cloudflare Tunnel（x001a）

自宅の **auth / world / assets**（`127.0.0.1:4002` / `4003` / `4004`）を、ルーターのポート開放なしで HTTPS 公開する。

正本 Slice: [`.workspace/roadmap/slices/x001a-https-edge-tunnel.md`](../../.workspace/roadmap/slices/x001a-https-edge-tunnel.md)

## 保証する / しない

| 保証する | 保証しない |
|:---|:---|
| Named Tunnel + ホスト名振り分けのテンプレ | Quick Tunnel・path 振り分け |
| 公網ポート開放ゼロで 3 サービスへ HTTPS 到達 | Zenoh WAN（→ x001b） |
| credentials / 実 `config.yml` をリポに入れない | DNS / Zone の完全自動化 |
| WSL から `127.0.0.1:400x` へ届く前提の起動手順 | cloudflared HA・複数拠点 |

## 前提

- Cloudflare アカウントと **Zone**（カスタムドメイン。DNS 手作業可）
- 自宅で auth / world / assets の Docker が起動済み（既存 `BIND_ALL`）
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

### 5. 自宅サービス起動

```bash
cd auth-server && docker compose up -d && cd ..
cd world-server && docker compose up -d && cd ..
cd assets-server && docker compose up -d && cd ..
```

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
- 自宅 host + 自宅 zenohd は従来どおり（本 Slice では Zenoh を変えない）
