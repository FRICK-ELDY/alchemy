# x001a — HTTPS 入口（Cloudflare Tunnel）

> **ID:** `x001a` · **slug:** `https-edge-tunnel` · **status:** `done` · **area:** `[auth, world, assets, docs, relay]` · **depends_on:** `[x0019]`

目標: 拠点の **auth / world / assets** を、ルーターのポート開放なしで **HTTPS 公開**する。外のクライアントが `AUTH_URL` / `WORLD_URL` / `ASSETS_URL` だけで login → Room Token → アセット取得まで縦に通ること。Zenoh WAN は次 Slice（x001b）。

正本: [overview.md](../../0_doc/architecture/overview.md)（Cloudflare Tunnel / 拠点 IP 秘匿）· [auth-server.md](../../0_doc/architecture/auth-server.md) · [world-server.md](../../0_doc/architecture/world-server.md) · [assets-server.md](../../0_doc/architecture/assets-server.md)

## 設計方針

| 層 | 役割 |
|:---|:---|
| cloudflared | 拠点から外向き Tunnel。`auth` / `world` / `assets` をホスト名（または path）で振り分け |
| Docker | 既存 `BIND_ALL` のまま localhost（または LAN）へ待受。公網ポートは開けない |
| 外 client | `AUTH_URL` / `WORLD_URL` / `ASSETS_URL` を `https://…` に差し替え |
| 拠点 host | 引き続き `http://127.0.0.1:400x` でよい（Tunnel 必須ではない） |

| Phase | 内容 | 状態 |
|:---|:---|:---|
| 0 | Slice 文書 + 目次 | done |
| 1 | `edge/cloudflared`（または同等）設定テンプレ + 起動 runbook | done |
| 2 | ホスト名振り分け（auth / world / assets）+ health 確認手順 | done |
| 3 | 外クライアント用 env 例 + README / architecture 追記 | done |
| 4 | smoke（Tunnel 経由 login → Room Token → assets health/CAS 1本） | done |

## 保証する / しない

| 保証する | 保証しない |
|:---|:---|
| ポート開放ゼロで auth / world / assets に HTTPS 到達 | Zenoh 公網待受・ConoHa リレー |
| 外 client が Tunnel URL で login → Room Token 取得 | WAF カスタムルール本格・Bot 対策の作り込み |
| 外 client が Tunnel URL で assets 取得（既存 CAS） | R2 / CDN 本配信・Quota 課金連動 |
| 拠点 IP をクライアント設定に書かない | カスタムドメイン自動化の完全運用（DNS 手作業可） |
| runbook と設定テンプレをリポに残す | cloudflared の HA・複数拠点 |

## やらない（後続）

- ConoHa zenohd / `ZENOH_CONNECT` WAN（→ **x001b**）
- リレー JWT ACL / Relay Allocation Token
- launcher / verifier / アバター GLB / 予測補間
- assets → R2 切替

## 成功条件（デモ）

1. 拠点で auth / world / assets と cloudflared が起動している
2. **別ネットワーク**（例: モバイルテザリング）のマシンから:
   - `GET https://<auth-host>/health`
   - login → world で Room Token
   - `GET https://<assets-host>/health`（または CAS 1 blob）
3. 拠点 host + 拠点 zenohd は従来どおり（本 Slice では Zenoh を変えない）

## 起動

Windows 手順書: [`manual/cloudflare-tunnel-windows.md`](../../0_doc/manual/cloudflare-tunnel-windows.md)  
テンプレ / 短い runbook: [`edge/cloudflared/README.md`](../../../edge/cloudflared/README.md)

```bash
# 拠点: 既存 Docker
cd auth-server && docker compose up -d && cd ..
cd world-server && docker compose up -d && cd ..
cd assets-server && docker compose up -d && cd ..

# Tunnel（config.yml は example からコピー・credentials 配置後）
cd edge/cloudflared && cloudflared tunnel --config config.yml run

# 外クライアント例（ホスト名は runbook の値に置換）
export AUTH_URL=https://auth.example.tld
export WORLD_URL=https://world.example.tld
export ASSETS_URL=https://assets.example.tld
# Zenoh はまだ LAN / localhost（x001b まで）
# bash .scripts/smoke-https-edge.sh
```

## 実装メモ（方針）

- ネイティブ client（ureq）なのでブラウザ CORS は本 Slice の対象外
- world / assets の `AUTH_JWKS_URL` は **Docker 内**のまま（`host.docker.internal:4002`）。Tunnel は外向け入口だけ
- JWT / Room Token の `iss` / 鍵は変えない。変わるのは到達 URL のみ
- 秘密（Tunnel token / 証明書）はリポに入れない。`.env.example` と gitignore で indirection

← [目次](../README.md)
