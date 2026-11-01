# relay-server WAN 手順（ConoHa / Ubuntu 22.04・24.04）

ConoHa VPS 上の **relay-server**（`zenohd`・**TLS+mTLS**・`7447`）を待受し、拠点 host と外 client が同じ `ROOM_ID` / `wire_cap` に入る。短命 client 証明書は **world-server が署名**する（[x0066](../../roadmap/slices/x0066-ephemeral-mtls-world-issue.md)）。

| | 役割 |
|:---|:---|
| **relay-server** | zenohd の設定・待受（空間同期の仲介）。client CA 証明書で mTLS を検証する |
| **world-server** | CSR を受け、私有 client CA で短命 mTLS 証明書を署名する（CN は role 由来） |
| **relay-agent** | 互換フォールバック。world が `mtls` を返さないときだけ Allocation Token から証明書を出す |

前提 Slice: [x001a HTTPS](../../roadmap/slices/x001a-https-edge-tunnel.md) · [x001b](../../roadmap/slices/x001b-zenoh-wan-conoha.md) · **TLS: [x0022](../../roadmap/slices/x0022-liveliness-wan-tls.md)** · **Relay Agent: [x004e](../../roadmap/slices/x004e-relay-agent-mvp.md)** · **world 短命 mTLS: [x0066](../../roadmap/slices/x0066-ephemeral-mtls-world-issue.md)**  
config: [`relay-server/config.wan.json5`](../../../relay-server/config.wan.json5)  
DNS / Let's Encrypt（サーバ証明書のドメイン化）: [`relay-server-dns.md`](relay-server-dns.md)

**推奨 WAN**: `ZENOH_CONNECT=tls/…` の host / client がオンメモリで CSR を作り、`POST /api/v1/rooms/:room_id/tokens` に `csr_pem` を付ける。world は **CSR の Subject を使わず**、role から CN（`alchemy-host` / `alchemy-guest`）で署名して `mtls` を返す。zenohd の `root_ca_certificate` は、その署名に使った **同一 client CA の証明書**で検証する。`RELAY_ALLOCATE_URL` は互換経路で、world が `mtls` を返したときは allocate しない。

想定スペック: **Ubuntu 22.04 または 24.04**、4Core / 4GB RAM（手順は両 LTS で同じ）。

## 二重の壁

1. **第1壁 — Zenoh ACL（CN）**: guest（`alchemy-guest`）は `frame` / `manifest` 等へ put できない。`input` への put はできる（[`config.wan.json5`](../../../relay-server/config.wan.json5)）
2. **第2壁 — `wire_cap`**: キー式 `alchemy/dev/room/{room_id}/{wire_cap}/…` と Room Token / アプリ署名

zenohd は **JWT を見ない**。平文 TCP は待受しない。`quic/` 待受は対象外。

## 構成

| 役割 | 配置 |
|:---|:---|
| auth / world / assets | 拠点（x001a Tunnel 可） |
| client CA 秘密鍵 | **拠点 world**（`MTLS_CA_KEY_PATH` または `MTLS_CA_KEY_PEM`） |
| client CA 証明書 | 拠点 world と **VPS zenohd**（`root_ca_certificate`）で同じファイル |
| host-server | 拠点。`ZENOH_CONNECT=tls/<VPS_HOST>:7447`（`tls/` なら CSR を自動で付ける） |
| 外 client | x001a の `https://…` + 同じ `ZENOH_CONNECT` |
| relay-server（zenohd） | **VPS**（mTLS + role ACL） |
| relay-agent | 任意。world に CA が無いときの互換（例 `:7450`） |

client の秘密鍵は配布物に同梱しない。接続のたびに手元で作り、一時 PEM に書く（Unix では `0600`）。

---

## 初回セットアップ

### 1. 証明書を分ける

開発用（私有サーバ証明書）はリポジトリのスクリプト。Let's Encrypt を使うときはサーバ証明書だけ [`relay-server-dns.md`](relay-server-dns.md) に合わせ、**client CA は私有のまま**残す。

```bash
bash relay-server/scripts/gen-wan-dev-certs.sh <VPS_IP_or_DNS> /tmp/alchemy-tls
```

| ファイル | 置く場所 | 用途 |
|:---|:---|:---|
| `server-cert.pem` / `server-key.pem` | VPS `/etc/alchemy/tls/` | zenohd の listen。Let's Encrypt のときは fullchain / privkey に差し替える |
| `client-ca-cert.pem` | VPS `/etc/alchemy/tls/` **と** 拠点 world | zenohd の `root_ca_certificate`。world の `MTLS_CA_CERT_PATH` |
| `client-ca-key.pem` | 拠点 world（`0600`） | `MTLS_CA_KEY_PATH`。world が CSR に署名する |
| `ca-cert.pem` | 私有サーバ証明書のとき、host / client | `ZENOH_TLS_ROOT_CA`（zenohd のサーバ証明書を信頼する） |

Let's Encrypt のサーバ証明書では、client / host の `ZENOH_TLS_ROOT_CA` は不要（公開 CA）。

VPS へサーバ材料と client CA **証明書**だけを置く:

```bash
sudo mkdir -p /etc/alchemy/tls
sudo cp /tmp/alchemy-tls/server-cert.pem /tmp/alchemy-tls/server-key.pem \
  /tmp/alchemy-tls/client-ca-cert.pem /etc/alchemy/tls/
sudo chmod 600 /etc/alchemy/tls/server-key.pem
```

`client-ca-key.pem` は推奨経路では VPS に置かない。Relay Agent を併用するときだけ、後述の `RELAY_TLS_DIR` に証明書と鍵の両方を置く。

### 2. 拠点 world に署名 CA を渡す

world プロセスから読めるパス（または PEM そのもの）を設定する。秘密鍵をログやレスポンスに出さない。

```bash
# パス
MTLS_CA_CERT_PATH=/path/to/client-ca-cert.pem
MTLS_CA_KEY_PATH=/path/to/client-ca-key.pem

# または PEM をそのまま（どちらか一方）
# MTLS_CA_CERT_PEM=...
# MTLS_CA_KEY_PEM=...
```

署名は openssl CLI（Elixir に x509 パッケージは足さない）。`openssl` が PATH に無いときは `OPENSSL_BIN` で実行ファイルを指定する。未設定、または CA が読めないとき、world は `csr_pem` を無視して Token だけ返し、`mtls` フィールドは付けない。

API 上の `expires_at` は署名から **6 時間**。openssl の NotAfter は **1 日**。期限の 30 分前にセッションを自動で開き直す手順は、この文書の範囲外。Token を取り直すときに host / client は新しい CSR を付ける。

### 3. SSH と zenohd（VPS）

```bash
ssh ubuntu@<VPS_IP>
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh
source "$HOME/.cargo/env"
cargo install zenohd
zenohd --help | head
```

### 4. config 配置

```bash
sudo mkdir -p /etc/alchemy
sudo cp relay-server/config.wan.json5 /etc/alchemy/zenoh-wan.json5
# listen: tls/0.0.0.0:7447
# listen_private_key / listen_certificate … サーバ証明書
# root_ca_certificate: /etc/alchemy/tls/client-ca-cert.pem
# enable_mtls: true と access_control はそのまま
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

### 6. ufw

7447 の入場は mTLS 握手（client CA で署名された証明書が無い接続は zenohd が拒否する）。送信元 IP の固定リストは手順に含めない。

```bash
sudo ufw default deny incoming
sudo ufw default allow outgoing
sudo ufw allow OpenSSH
sudo ufw allow 7447/tcp
sudo ufw enable
sudo ufw status numbered
```

すでに「特定のグローバル IP だけ 7447」というルールがある VPS は、そのルールを外して `ufw allow 7447/tcp` にする。限定ルールが残っていると、登録外のクライアントは握手の前に捨てられる。

ベンダーのセキュリティグループも **7447/tcp をクライアントから到達できる**ようにする。SSH は送信元を絞ったままにする。平文の publish ポートは増やさない。

到達確認:

```bash
nc -vz <VPS_IP> 7447
```

`bash .scripts/smoke-zenoh-wan.sh` は TCP 待受の確認であり、TLS 握手は見ない。

### 7. Relay Agent（任意・互換）

world に `MTLS_CA_*` が無い既存構成向け。推奨経路（world が `mtls` を返す）では起動しない。

```bash
cd relay-agent
cargo build --release
# WORLD_JWKS_URL=https://world…/.well-known/jwks.json
# RELAY_TLS_DIR=/etc/alchemy/tls   # client-ca-cert.pem と client-ca-key.pem
# RELAY_AGENT_LISTEN=127.0.0.1:7450
./target/release/alchemy-relay-agent
```

host / client に `RELAY_ALLOCATE_URL=http://<到達できるホスト>:7450` を付ける。待受を VPS の外に出すときは、7450 も送信元を絞る。world が `mtls` を返した接続は、この URL が設定されていても allocate しない。

---

## クライアント起動

`ZENOH_CONNECT` が `tls/` で始まる（または `WORLD_MTLS=1`）とき、host / client は Room Token 要求に CSR を付ける。事前に client 証明書を配る手順は無い。

### 拠点 host

world / auth が同じマシンの既定（`:4003` / `:4002`）なら URL は省略できる。world に §2 の CA が入っていること。

```powershell
$env:ZENOH_CONNECT = "tls/<VPS_HOST>:7447"
# 私有サーバ証明書のときだけ:
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
# 私有サーバ証明書のときだけ:
# $env:ZENOH_TLS_ROOT_CA = "D:\path\to\ca-cert.pem"
$env:ROOM_ID = "local"
$env:ALCHEMY_FORCE_DESKTOP = "1"
cd D:\github\frick\alchemy\client
cargo run -p client
```

2 台目も同じ `ROOM_ID` / `ZENOH_CONNECT` で起動。logout → Leave で即時 reclaim（x0019）。liveliness 偽 Delete は jti 不一致で無視（x0022）。

```bash
export ZENOH_CONNECT=tls/<VPS_HOST>:7447
# export ZENOH_TLS_ROOT_CA=/path/to/ca-cert.pem   # 私有サーバ証明書のとき
bash .scripts/smoke-zenoh-wan.sh
```

---

## 毎回の運用

1. 拠点 world が `MTLS_CA_CERT_PATH` と `MTLS_CA_KEY_PATH`（または PEM）を読める  
2. VPS: `systemctl status alchemy-zenohd` が active  
3. 拠点: auth / world / assets（+ 必要なら cloudflared）  
4. 拠点: `ZENOH_CONNECT=tls/<VPS_HOST>:7447` で host-server  
5. 外: x001a URL + 同じ `ZENOH_CONNECT` で client  

---

## トラブルシュート

| 症状 | 見ること |
|:---|:---|
| TLS handshake 失敗 | サーバ証明書の CN/SAN と `ZENOH_CONNECT` のホスト名。私有サーバ CA なら `ZENOH_TLS_ROOT_CA`。client 証明書拒否なら、world の CA と zenohd の `root_ca_certificate` が同じ `client-ca-cert.pem` か、world が `mtls` を返しているか |
| Token は取れるが zenoh に入れない | world ログの `MTLS CA not configured` / `unreadable`。`csr_pem` が無視されると `mtls` が無く、`RELAY_ALLOCATE_URL` も無ければ client 証明書が無い |
| connection refused | zenohd / `ss -lntp \| grep 7447`。ufw とセキュリティグループが 7447/tcp を許可しているか。平文 `tcp/` で繋いでいないか |
| 特定の回線だけ届かない | 古い送信元 IP 限定の 7447 ルールが残っていないか |
| guest の frame が拒否される | 想定どおり（第1壁）。CN が `alchemy-guest` なら `input` は put でき、`frame` はできない |
| HTTPS は通るが空間が空 | `ZENOH_CONNECT` / `ROOM_ID` の一致。第2壁は `wire_cap` |

---

## 保証する / しない

| 保証する | 保証しない |
|:---|:---|
| WAN 待受が TLS+mTLS（平文 TCP の公開をしない） | QUIC / WebTransport / P2P |
| world が CSR を受け、Subject を使わず role CN で短命 cert を署名する | 期限 30 分前の完全な自動セッション再 open |
| zenohd と world が同一 client CA を検証と署名に使う | ACL ホットリロード / per-room subject |
| role CN ACL（第1壁）と `wire_cap`（第2壁） | Billing / CCU・帯域 QoS |
| ローカル `tcp/` と、CA 未設定時の Relay Agent 経路 | 既存 VPS の IP 限定ルール撤去や Steam 公開面 Tunnel の移行作業そのもの |
| x001a HTTPS と併用した外からの入室 | 端末に置いた一時 PEM の窃取 |
| liveliness Delete が検証済み jti に紐付く（x0022） | client 秘密鍵の永続同梱 |
