# Zenoh WAN 手順（ConoHa / Ubuntu 22.04・24.04）

ConoHa VPS 上の **zenohd**（**TLS+mTLS**・`7447`）と **Relay Agent**（Allocation → 短命 client cert）を待受し、拠点 host と外 client が同じ `ROOM_ID` / `wire_cap` に入る。

前提 Slice: [x001a HTTPS](../../roadmap/slices/x001a-https-edge-tunnel.md) · [x001b](../../roadmap/slices/x001b-zenoh-wan-conoha.md) · **TLS: [x0022](../../roadmap/slices/x0022-liveliness-wan-tls.md)** · **mTLS/Relay: [x004e](../../roadmap/slices/x004e-relay-agent-mvp.md)**  
config: [`zenoh-router/config.wan.json5`](../../../zenoh-router/config.wan.json5) · agent: [`relay-agent/`](../../../relay-agent/)

想定スペック: **Ubuntu 22.04 または 24.04**、4Core / 4GB RAM（手順は両 LTS で同じ）。

## 構成

| 役割 | 配置 |
|:---|:---|
| auth / world / assets | 拠点（x001a Tunnel 可） |
| host-server | 拠点。`ZENOH_CONNECT=tls/<VPS_HOST>:7447` + `RELAY_ALLOCATE_URL` |
| 外 client | x001a の `https://…` + 同じ Zenoh / Relay env |
| zenohd | **VPS**（mTLS + role ACL） |
| Relay Agent | **VPS**（`POST /v1/allocate`、例 `:7450`） |

zenohd は **JWT を見ない**。Allocation Token は Relay Agent が検証する（x004e）。平文 TCP は待受しない。

---

## 初回セットアップ（VPS）

### 1. SSH

```bash
ssh ubuntu@<VPS_IP>
```

### 2. zenohd の導入

```bash
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh
source "$HOME/.cargo/env"
cargo install zenohd
zenohd --help | head
```

### 3. TLS 証明書

**A. 開発用（私有 CA）** — IP 直打ちや検証用。client / host に CA を配る。

手元または VPS で:

```bash
# リポジトリから
bash zenoh-router/scripts/gen-wan-dev-certs.sh <VPS_IP_or_DNS> /tmp/alchemy-tls
sudo mkdir -p /etc/alchemy/tls
sudo cp /tmp/alchemy-tls/server-cert.pem /tmp/alchemy-tls/server-key.pem \
  /tmp/alchemy-tls/client-ca-cert.pem /tmp/alchemy-tls/client-ca-key.pem /etc/alchemy/tls/
sudo chmod 600 /etc/alchemy/tls/server-key.pem /etc/alchemy/tls/client-ca-key.pem
# ca-cert.pem を拠点・外 client に渡し ZENOH_TLS_ROOT_CA に設定
```

### 3b. Relay Agent

```bash
cd relay-agent
cargo build --release
# WORLD_JWKS_URL=https://world…/.well-known/jwks.json
# RELAY_TLS_DIR=/etc/alchemy/tls
# RELAY_AGENT_LISTEN=0.0.0.0:7450   # ufw で allowlist
./target/release/alchemy-relay-agent
```


**B. 本番寄り（Let's Encrypt）** — 公開 DNS 名があるとき。`listen_certificate` / `listen_private_key` を certbot の fullchain/privkey に合わせる。client 側の `ZENOH_TLS_ROOT_CA` は不要（webpki 既定）。

### 4. config 配置

```bash
sudo mkdir -p /etc/alchemy
sudo cp zenoh-router/config.wan.json5 /etc/alchemy/zenoh-wan.json5
# listen: tls/0.0.0.0:7447
# transport.link.tls.listen_* が /etc/alchemy/tls/… を指すこと
```

### 5. systemd

`/etc/systemd/system/alchemy-zenohd.service` 例:

```ini
[Unit]
Description=Alchemy zenohd (WAN TLS)
After=network-online.target
Wants=network-online.target

[Service]
Type=simple
ExecStart=/home/ubuntu/.cargo/bin/zenohd -c /etc/alchemy/zenoh-wan.json5
Restart=on-failure
RestartSec=3

[Install]
WantedBy=multi-user.target
```

```bash
sudo systemctl daemon-reload
sudo systemctl enable --now alchemy-zenohd
sudo systemctl status alchemy-zenohd
```

### 6. ufw（allowlist）

**全世界へ 7447 を開けない。**

```bash
sudo ufw default deny incoming
sudo ufw default allow outgoing
sudo ufw allow OpenSSH
sudo ufw allow from <SITE_GLOBAL_IP> to any port 7447 proto tcp
sudo ufw allow from <CLIENT_GLOBAL_IP> to any port 7447 proto tcp
sudo ufw enable
sudo ufw status numbered
```

到達確認（allowlist 内）:

```bash
nc -vz <VPS_IP> 7447
```

---

## クライアント起動

### 拠点 host

```powershell
$env:ZENOH_CONNECT = "tls/<VPS_HOST>:7447"
# 私有 CA のときのみ:
# $env:ZENOH_TLS_ROOT_CA = "D:\path\to\ca-cert.pem"
cd D:\github\frick\alchemy\client
cargo run -p host-server
```

### 外 client（別ネットワーク）

```powershell
$env:AUTH_URL = "https://auth.example.tld"
$env:WORLD_URL = "https://world.example.tld"
$env:ASSETS_URL = "https://assets.example.tld"
$env:ZENOH_CONNECT = "tls/<VPS_HOST>:7447"
# $env:ZENOH_TLS_ROOT_CA = "D:\path\to\ca-cert.pem"
$env:ROOM_ID = "local"
$env:ALCHEMY_FORCE_DESKTOP = "1"
cd D:\github\frick\alchemy\client
cargo run -p client
```

2 台目も同じ `ROOM_ID` / `ZENOH_CONNECT` で起動。logout → Leave で即時 reclaim（x0019）。liveliness 偽 Delete は jti 不一致で無視（x0022）。

```bash
export ZENOH_CONNECT=tls/<VPS_HOST>:7447
# export ZENOH_TLS_ROOT_CA=/path/to/ca-cert.pem
bash .scripts/smoke-zenoh-wan.sh
```

---

## 毎回の運用

1. VPS: `systemctl status alchemy-zenohd` が active  
2. 拠点: auth / world / assets（+ 必要なら cloudflared）  
3. 拠点: `ZENOH_CONNECT=tls/<VPS_HOST>:7447` で host-server  
4. 外: x001a URL + 同じ `ZENOH_CONNECT` で client  

---

## トラブルシュート

| 症状 | 見ること |
|:---|:---|
| TLS handshake 失敗 | CN/SAN と `ZENOH_CONNECT` のホスト名が一致するか。私有 CA なら `ZENOH_TLS_ROOT_CA` |
| client が繋がらない | ufw に今のグローバル IP が入っているか |
| connection refused | zenohd / `ss -lntp \| grep 7447`。平文 `tcp/` で繋いでいないか |
| HTTPS は通るが空間が空 | `ZENOH_CONNECT` / `ROOM_ID` 一致 |

---

## 保証する / しない

| 保証する | 保証しない |
|:---|:---|
| WAN 待受が TLS+mTLS（平文 TCP のみの公開をしない） | Billing / 第三者リレー発見 |
| Relay Agent が Allocation Token 検証後にだけ client cert を発行 | CCU / 帯域 QoS enforcement |
| role CN ACL + `wire_cap` による他 room キー到達拒否 | UI 後付け join のセッション再 open |
| IP allowlist による公開面の最小化 | QUIC / WebTransport / P2P |
| x001a HTTPS と併用した外からの入室手順 | ACL ホットリロード |
| liveliness Delete が検証済み jti に紐付く（x0022） | 端末秘密鍵窃取 |
