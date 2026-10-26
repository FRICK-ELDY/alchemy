# Alchemy

分散連合型 VR プラットフォーム（目標アーキテクチャ: [`.workspace/0_doc/architecture/overview.md`](.workspace/0_doc/architecture/overview.md)）。

**対応 OS:** macOS / Linux / **Windows は WSL2（Ubuntu 26.04 LTS）**  
ネイティブ Windows での `cargo` はアプリケーション制御（os error 4551）の都合で推奨しません。

## 現状（x001a / HTTPS 入口）

`host-server` は **Rapier 60Hz**・Room Token **`sub` 別** Intent/Grab（最大 **4**）・**user_root entity 3..6**・**PresenceState**・**Leave / liveliness / Intent TTL 解放**・**`ROOM_ID` Zenoh キー**・**立方体 ×9**・**30Hz デルタ**。  
`client` は Dash 下タブ **Home / Settings / Exit**。Home で **login / 最小 register / logout**（logout は Leave + liveliness undeclare）。未ログインでは Room Token なし。`AUTH_USER` / `ALCHEMY_AUTH_TOKEN` は CI サイレント login。Desktop Grab / XR Grab / Noclip は従来どおり。  
**LAN:** zenohd `0.0.0.0:7447`、**`ZENOH_CONNECT`**。  
**HTTPS:** Cloudflare Tunnel で auth / world / assets を公開（[`edge/cloudflared/`](edge/cloudflared/)）。  
**次:** [x001b Zenoh WAN](.workspace/roadmap/slices/x001b-zenoh-wan-conoha.md)。空き ID = **`x001c`**。[roadmap](.workspace/roadmap/README.md)。

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

## 起動（macOS / Linux / WSL）

```bash
# 1) auth + world + assets
cd auth-server && docker compose up -d && cd ..
cd world-server && docker compose up -d && cd ..
cd assets-server && docker compose up -d && cd ..

# 2) zenohd
zenohd -c zenoh-router/config.json5

# 3) host + client（別ターミナル）
cd client
cargo run -p host-server
# UI login: omit AUTH_USER → Home tab opens
ALCHEMY_FORCE_DESKTOP=1 cargo run -p client
# CI silent:
AUTH_USER=alice ALCHEMY_FORCE_DESKTOP=1 cargo run -p client

# 4) smoke（任意）
bash .scripts/smoke-assets.sh
bash .scripts/smoke-assets-glb.sh      # Coin GLB → CAS
bash .scripts/smoke-manifest-delta.sh  # Manifest + デルタ（zenohd 必須）
bash .scripts/smoke-rapier-host.sh     # Rapier 60Hz / net 30Hz（zenohd 必須）
bash .scripts/smoke-system-ui.sh       # system_ui unit + host_control コンパイル
bash .scripts/smoke-openxr.sh          # OpenXR モジュール unit + Desktop フォールバック compile
bash .scripts/smoke-desktop-noclip.sh  # Manifest Noclip + user_root（zenohd + auth/world 必須）
bash .scripts/smoke-lan-two-client.sh   # legacy x0014 units (still useful)
bash .scripts/smoke-presence-slots.sh   # Presence slots + ROOM_ID keys + pin banner
bash .scripts/smoke-client-auth.sh      # x0018 Home units + login→logout HTTP
bash .scripts/smoke-leave-liveliness.sh # x0019 Leave + liveliness units + host banner
bash .scripts/smoke-https-edge.sh       # x001a Tunnel: AUTH/WORLD/ASSETS_URL 必須
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
#   Zenoh はまだ LAN / localhost（x001b まで）。公網ポート開放は不要。
# ESC ダッシュ / Home タブ / XR Grab / Noclip。Desktop 強制: ALCHEMY_FORCE_DESKTOP=1
# OpenXR ローダー明示: ALCHEMY_OPENXR_LOADER=...
```

環境変数（任意）: `AUTH_URL` / `WORLD_URL` / `ASSETS_URL` / `ROOM_ID`（既定 `local`） / `AUTH_USER` / `AUTH_PASSWORD` / `ALCHEMY_AUTH_TOKEN`（サイレント login） / `ZENOH_CONNECT` / `ALCHEMY_BLOB_BLAKE3` / `ALCHEMY_HOST_PID` / `ALCHEMY_FORCE_DESKTOP` / `ALCHEMY_OPENXR_LOADER` / `STEAM_PATH`。

詳細は [`.workspace/roadmap/`](.workspace/roadmap/README.md)。

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
