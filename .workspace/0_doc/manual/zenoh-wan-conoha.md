# Zenoh WAN 手順（ConoHa / Ubuntu 22.04・24.04）

ConoHa VPS 上の **zenohd だけ**を公網 TCP（`7447`）で待受し、拠点 host と外 client が `ZENOH_CONNECT` で同じ `ROOM_ID` に入る。

前提 Slice: [x001a HTTPS](../../roadmap/slices/x001a-https-edge-tunnel.md) · 本 Slice: [x001b](../../roadmap/slices/x001b-zenoh-wan-conoha.md)  
config: [`zenoh-router/config.wan.json5`](../../../zenoh-router/config.wan.json5)

想定スペック: **Ubuntu 22.04 または 24.04**、4Core / 4GB RAM（手順は両 LTS で同じ）。

## 構成

| 役割 | 配置 |
|:---|:---|
| auth / world / assets | 拠点（x001a Tunnel 可） |
| host-server | 拠点。`ZENOH_CONNECT=tcp/<VPS_IP>:7447` |
| 外 client | x001a の `https://…` + 同じ `ZENOH_CONNECT` |
| zenohd | **VPS のみ**（拠点のローカル zenohd は使わない） |

zenohd は **JWT を見ない**。覗き見・ノイズは **ufw IP allowlist** で抑える。

---

## 初回セットアップ（VPS）

### 1. SSH

```bash
ssh ubuntu@<VPS_IP>
# または ConoHa で作ったユーザー
```

### 2. zenohd の導入

Rust ツールチェーンがある場合:

```bash
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh
source "$HOME/.cargo/env"
cargo install zenohd
zenohd --help | head
```

バイナリを `/usr/local/bin/zenohd` に置く場合は、そのパスを後述の systemd に合わせる。

### 3. config 配置

リポジトリの WAN config をコピーする（VPS 上に clone するか、手元から `scp`）。

```bash
sudo mkdir -p /etc/alchemy
sudo cp zenoh-router/config.wan.json5 /etc/alchemy/zenoh-wan.json5
# 内容確認: listen tcp/0.0.0.0:7447、multicast scouting disabled
```

### 4. systemd

`/etc/systemd/system/alchemy-zenohd.service` 例:

```ini
[Unit]
Description=Alchemy zenohd (WAN)
After=network-online.target
Wants=network-online.target

[Service]
Type=simple
ExecStart=/home/ubuntu/.cargo/bin/zenohd -c /etc/alchemy/zenoh-wan.json5
Restart=on-failure
RestartSec=3
# User=ubuntu
# 必要なら LimitNOFILE=65535

[Install]
WantedBy=multi-user.target
```

`ExecStart` の zenohd パスは `which zenohd` の結果に合わせる。

```bash
sudo systemctl daemon-reload
sudo systemctl enable --now alchemy-zenohd
sudo systemctl status alchemy-zenohd
```

### 5. ufw（allowlist）

**全世界へ 7447 を開けない。** 拠点のグローバル IP と、検証用 client（例: スマホ回線の一時 IP）だけ許可する。

```bash
sudo ufw default deny incoming
sudo ufw default allow outgoing
sudo ufw allow OpenSSH
# 拠点（例）
sudo ufw allow from <SITE_GLOBAL_IP> to any port 7447 proto tcp
# 検証 client（例・変わるときは都度更新）
sudo ufw allow from <CLIENT_GLOBAL_IP> to any port 7447 proto tcp
sudo ufw enable
sudo ufw status numbered
```

実 IP はリポに書かない。手元メモや VPS 上のメモのみ。

到達確認（allowlist 内のマシンから）:

```bash
# TCP だけ見る例
nc -vz <VPS_IP> 7447
```

---

## クライアント起動

### 拠点 host

HTTPS は localhost でよい。Zenoh だけ VPS へ。

```powershell
# Windows 例
$env:ZENOH_CONNECT = "tcp/<VPS_IP>:7447"
cd D:\github\frick\alchemy\client
cargo run -p host-server
```

### 外 client（別ネットワーク）

```powershell
$env:AUTH_URL = "https://auth.example.tld"
$env:WORLD_URL = "https://world.example.tld"
$env:ASSETS_URL = "https://assets.example.tld"
$env:ZENOH_CONNECT = "tcp/<VPS_IP>:7447"
$env:ROOM_ID = "local"
$env:ALCHEMY_FORCE_DESKTOP = "1"
cd D:\github\frick\alchemy\client
cargo run -p client
```

env 例: [`edge/client.env.example`](../../../edge/client.env.example)

2 台目も同じ `ROOM_ID` / `ZENOH_CONNECT` で起動し、互いの user_root が見えること。logout → Leave で即時 reclaim（x0019）も WAN で維持されること。

軽い接続確認:

```bash
export ZENOH_CONNECT=tcp/<VPS_IP>:7447
bash .scripts/smoke-zenoh-wan.sh
```

---

## 毎回の運用

1. VPS: `systemctl status alchemy-zenohd` が active  
2. 拠点: auth / world / assets（+ 必要なら cloudflared）  
3. 拠点: `ZENOH_CONNECT=tcp/<VPS_IP>:7447` で host-server  
4. 外: x001a URL + 同じ `ZENOH_CONNECT` で client  

拠点側でローカル `zenohd`（`config.json5`）を並行起動する必要はない（WAN デモ時）。

---

## トラブルシュート

| 症状 | 見ること |
|:---|:---|
| client が繋がらない | ufw に今のグローバル IP が入っているか（回線変更で IP が変わる） |
| connection refused | zenohd / systemd が落ちていないか、`ss -lntp \| grep 7447` |
| HTTPS は通るが空間が空 | host と client の `ZENOH_CONNECT` / `ROOM_ID` が一致しているか |
| ローカルでは動くが WAN だけダメ | 拠点から `nc -vz <VPS_IP> 7447`、VPS セキュリティグループ |

---

## 保証する / しない

| 保証する | 保証しない |
|:---|:---|
| VPS zenohd 経由で同一 `ROOM_ID` の pub/sub | Room Token による zenohd ACL |
| IP allowlist による公開面の最小化 | QUIC / WebTransport / P2P |
| x001a HTTPS と併用した外からの入室手順 | CCU / 帯域 QoS・Relay Allocation Token |
