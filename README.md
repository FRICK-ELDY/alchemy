# Alchemy

分散連合型 VR プラットフォーム（目標アーキテクチャ: [`.workspace/0_doc/architecture/overview.md`](.workspace/0_doc/architecture/overview.md)）。

**対応 OS:** macOS / Linux / **Windows は WSL2（Ubuntu 26.04 LTS）を日常開発の正本**  
ネイティブ Windows の日常 `cargo` はアプリケーション制御（os error 4551）の都合で推奨しません。  
**テスター配布 ZIP**（ネイティブ Windows x64）は [Releases（Pre-release）](https://github.com/FRICK-ELDY/alchemy/releases) · 手順 [windows-tester-zip.md](.workspace/0_doc/manual/windows-tester-zip.md)。版名は `v0.0.0-dev.YYYYMMDDHHMM`（JST）。

## 現状（x0020 / Desktop client 手通し）

`host-server` は **Rapier 60Hz**・Room Token **`sub` 別** Intent/Grab（最大 **4**）・**user_root entity 3..6**・**PresenceState**・**Leave / liveliness / Intent TTL 解放**・**`ROOM_ID` Zenoh キー**・**立方体 ×9**・**30Hz デルタ**。  
`client` は Dash 下タブ **Home / Settings / Exit**。Home で **login / 最小 register / logout**（logout は Leave + liveliness undeclare）。未ログインでは Room Token なし。`AUTH_USER` / `ALCHEMY_AUTH_TOKEN` は CI サイレント login。Desktop Grab / XR Grab / Noclip は従来どおり。  
**Desktop 手通し（x0020）:** 日常スタック（`:4002/:4003/:4004` + zenohd `:7447`）が正本。手順は下記「Desktop 手通し」、env 例は [`client/env.desktop.example`](client/env.desktop.example)、機械検証は `make smoke-desktop-handthrough`。  
**LAN:** zenohd `0.0.0.0:7447`、**`ZENOH_CONNECT`**。  
**HTTPS（x001a）:** Cloudflare Tunnel で auth / world / assets を公開（[`edge/cloudflared/`](edge/cloudflared/)）。  
**WAN（x001b）:** ConoHa 上 zenohd + IP allowlist（[`zenoh-router/config.wan.json5`](zenoh-router/config.wan.json5) · [手順書](.workspace/0_doc/manual/zenoh-wan-conoha.md)）。  
**Assets（x001c/x001d）:** CAS を local / **R2** 切替。s3 時 GET は **署名 URL 302**（client → R2 直 DL）。  
**Hermetic E2E（x001f）:** `docker-compose.e2e.yml`（`:4102` 系 / Zenoh `:7547`）。Desktop をそちらへ向ける場合は env 例のコメントを参照。  
次の空き ID = **`x0046`**（P2-1 mTLS/ACL は deferred）。[roadmap](.workspace/roadmap/README.md)。

| コンポーネント | パス |
|:---|:---|
| auth-server | [`auth-server/`](auth-server/) |
| world-server | [`world-server/`](world-server/) |
| assets-server | [`assets-server/`](assets-server/) |
| zenoh-router | [`zenoh-router/`](zenoh-router/) |
| protocol（共有ワイヤ契約） | [`protocol/`](protocol/) |
| client workspace | [`client/`](client/)（`app` + `host-server`） |
| edge（Cloudflare Tunnel） | [`edge/cloudflared/`](edge/cloudflared/) |

ルートに Cargo workspace は置きません。Rust のビルド根は `client/`、契約クレートは独立した `protocol/` です。

## Windows: WSL2 + Ubuntu 26.04 LTS

### 1. ディストロの導入（PowerShell・管理者推奨）

```powershell
wsl --install -d Ubuntu-26.04
# すでに取得済みなら初回起動だけ:
wsl -d Ubuntu-26.04
```

初回起動で UNIX ユーザー名とパスワードを作成する。

既定ディストロにする（任意）:

```powershell
wsl --set-default Ubuntu-26.04
```

### 2. Docker Desktop 連携

Docker Desktop → Settings → Resources → **WSL Integration** で `Ubuntu-26.04` を有効化。

### 3. 開発ツール

WSL 内:

```bash
bash .scripts/wsl-setup.sh
```

`wsl-setup.sh` は build-essential / wgpu 用ライブラリ / rustup / zenohd を入れます。

### 4. Cursor / VS Code

リモート先を WSL の `Ubuntu-26.04` にし、リポジトリを WSL 側パスで開く（`\\wsl$\Ubuntu-26.04\home\...` へ clone する方が I/O は速い。`/mnt/d/...` でも可）。

## 前提ツール（WSL / macOS / Linux）

| ツール | 用途 | 備考 |
|:---|:---|:---|
| Rust (stable) | client / host-server / protocol | [rustup](https://rustup.rs/) または `wsl-setup.sh` |
| Docker / Docker Compose | auth / world / assets | Desktop + WSL Integration、または Linux ネイティブ |
| zenohd | ローカルルーター | `cargo install zenohd` |

任意: Elixir 1.19 / OTP 28（ホスト直実行したい場合のみ）。

Linux / WSL で `client` をビルドする場合の例（`wsl-setup.sh` に含む）:

```bash
sudo apt-get install -y pkg-config libxkbcommon-dev libwayland-dev \
  libx11-dev libxcursor-dev libxi-dev libxrandr-dev
```

## Desktop 手通し（x0020・正本）

日常スタック（`:4002/:4003/:4004` + zenohd `:7447`）で Desktop まで通す。env 一覧は [`client/env.desktop.example`](client/env.desktop.example)。

```bash
# 1) auth + world + assets
cd auth-server && docker compose up -d && cd ..
cd world-server && docker compose up -d && cd ..
cd assets-server && docker compose up -d && cd ..

# 2) zenohd
zenohd -c zenoh-router/config.json5

# --- 次はどちらか一方（同一 ROOM_ID で dual host にしない）---

# 3a) 手動手通し: host + Desktop client（別ターミナル）
cd client
cargo run -p host-server
# UI login: omit AUTH_USER → Home tab opens
ALCHEMY_FORCE_DESKTOP=1 cargo run -p client
# silent (debug は AUTH_USER のみ可 / release は password または token 必須):
AUTH_USER=alice AUTH_PASSWORD=Password1 ALCHEMY_FORCE_DESKTOP=1 cargo run -p client
# または: ALCHEMY_AUTH_TOKEN=<access_jwt> ALCHEMY_FORCE_DESKTOP=1 cargo run -p client

# 3a') launcher（x0032）: host+client を一括。client 終了後も host 維持。Ctrl+C で IPC stop
cd launcher && cargo run -p alchemy-launcher

# 3a'') Windows テスター ZIP（正本はタグ push → CI Pre-release）
#   powershell.exe -NoProfile -ExecutionPolicy Bypass -File .\.scripts\publish-windows-release.ps1 -CreateTag -PushTag
#   ローカル煙: powershell.exe -NoProfile -ExecutionPolicy Bypass -File .\.scripts\pack-windows-zip.ps1
#   手順: .workspace/0_doc/manual/windows-tester-zip.md

# 3b) 機械検証（3a の代替。既存 host を止めてから。スタック未起動は赤）
#    assert: host Token/Manifest/physics + client build/units（GUI・login→描画はしない）
make smoke-desktop-handthrough
```

Hermetic e2e（x001f）に Desktop を向ける場合は `env.desktop.example` のコメント（`:4102` 系 / Zenoh `:7547`）を export。GUI CI は必須化していない。

## 起動（その他 smoke / LAN / edge）

```bash
# smoke（任意）
make smoke-assets                      # または bash .scripts/smoke-assets.sh
bash .scripts/smoke-assets-glb.sh      # Coin GLB → CAS
make smoke-assets-r2                   # x001c/x001d: CAS=s3 + 302 presign（R2 env 要）
bash .scripts/smoke-manifest-delta.sh  # Manifest + デルタ（zenohd 必須）
bash .scripts/smoke-rapier-host.sh     # Rapier 60Hz / net 30Hz（zenohd 必須）
bash .scripts/smoke-system-ui.sh       # system_ui unit + host_control コンパイル
bash .scripts/smoke-openxr.sh          # OpenXR モジュール unit + Desktop フォールバック compile
bash .scripts/smoke-desktop-noclip.sh  # Manifest Noclip + user_root（zenohd + auth/world 必須）
make smoke-desktop-handthrough         # x0020: auth/world/zenoh + host Token/Manifest/physics + client build
bash .scripts/smoke-lan-two-client.sh   # legacy x0014 units (still useful)
bash .scripts/smoke-presence-slots.sh   # Presence slots + ROOM_ID keys + pin banner
bash .scripts/smoke-client-auth.sh      # x0018 Home units + login→logout HTTP
bash .scripts/smoke-leave-liveliness.sh # x0019 Leave + liveliness units + host banner
bash .scripts/smoke-https-edge.sh       # x001a Tunnel: AUTH/WORLD/ASSETS_URL 必須
bash .scripts/smoke-zenoh-wan.sh        # x001b WAN: ZENOH_CONNECT 必須（TCP 到達）
make ci-e2e                             # x001f hermetic: compose 縦 login→token→input→frame
#   前提: docker compose -f docker-compose.e2e.yml up -d --build
#   または COMPOSE_UP=1 make ci-e2e
# 3 client（同マシン・既定 ROOM_ID=local）:
#   ALCHEMY_FORCE_DESKTOP=1 AUTH_USER=alice cargo run -p client
#   ALCHEMY_FORCE_DESKTOP=1 AUTH_USER=bob cargo run -p client
#   ALCHEMY_FORCE_DESKTOP=1 AUTH_USER=carol cargo run -p client
# 別部屋: host/client とも ROOM_ID=alpha（beta とはキー隔離）。logout → 即時 reclaim（x0019）；切断のみは liveliness / TTL。
# LAN ノートPC: AUTH/WORLD/ASSETS_URL + ZENOH_CONNECT=tcp/<PC_IP>:7447 / AUTH_USER=bob|carol
#   （PC 側 Docker は BIND_ALL=true。Windows ファイアウォールで 7447 と 4002–4004 を許可）
# HTTPS（Tunnel・x001a）: 別ネットワークの client
#   手順: edge/cloudflared/README.md
#   cd edge/cloudflared && cloudflared tunnel --config config.yml run
#   export AUTH_URL=https://auth.example.tld WORLD_URL=https://world.example.tld ASSETS_URL=https://assets.example.tld
#   （env 例: edge/client.env.example / smoke: bash .scripts/smoke-https-edge.sh）
#   公網へのルーターポート開放は不要（Tunnel）。
# Zenoh WAN TLS（x0022 / ConoHa）:
#   手順: .workspace/0_doc/manual/zenoh-wan-conoha.md
#   VPS: zenohd -c config.wan.json5（tls）+ certs + ufw allowlist（7447 全世界開放禁止）
#   export ZENOH_CONNECT=tls/<VPS_HOST>:7447
#   # 私有 CA: export ZENOH_TLS_ROOT_CA=/path/to/ca-cert.pem
#   拠点 host + 外 client（x001a URL）で同じ ROOM_ID。smoke: bash .scripts/smoke-zenoh-wan.sh
# ESC ダッシュ / Home タブ / XR Grab / Noclip。Desktop 強制: ALCHEMY_FORCE_DESKTOP=1
# OpenXR ローダー明示: ALCHEMY_OPENXR_LOADER=...
```

環境変数（任意）: `AUTH_URL` / `WORLD_URL` / `ASSETS_URL` / `ROOM_ID`（既定 `local`） / `AUTH_USER` / `AUTH_PASSWORD` / `ALCHEMY_AUTH_TOKEN`（サイレント login） / `ZENOH_CONNECT` / `ZENOH_TLS_ROOT_CA`（WAN 私有 CA） / `ALCHEMY_BLOB_BLAKE3` / `ALCHEMY_HOST_PID` / `ALCHEMY_FORCE_DESKTOP` / `ALCHEMY_OPENXR_LOADER` / `STEAM_PATH`。Desktop 正本例: [`client/env.desktop.example`](client/env.desktop.example)。

詳細は [`.workspace/roadmap/`](.workspace/roadmap/README.md) · Slice [x0020](.workspace/roadmap/slices/x0020-desktop-handthrough.md)。

## ローカル CI

GitHub Actions と同じゲートを手元で実行します。**Elixir 系は全 OS で Docker 既定**。

```bash
chmod +x .scripts/ci.sh   # 初回のみ
make ci
make ci-rust
make ci-protocol
make ci-auth
make ci-world
make ci-assets
make ci-e2e   # x001f（compose スタック起動後）
make smoke-desktop-handthrough  # x0020（日常スタック起動後）
```

ホストの `mix` で auth を回す場合のみ:

```bash
AUTH_CI_HOST=1 ./.scripts/ci.sh auth
```

| Target | 内容 |
|:---|:---|
| `rust` | `client/` で fmt / clippy / build / test |
| `protocol` | `protocol/` で `cargo test`（`protoc` があれば schema も） |
| `auth` | Docker で format / compile / credo / ecto / `mix test` |
| `world` | Docker で format / compile / credo / `mix test` |
| `assets` | Docker で format / compile / credo / ecto / `mix test` |
