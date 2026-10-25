# Alchemy

分散連合型 VR プラットフォーム（目標アーキテクチャ: [`.workspace/0_doc/architecture/overview.md`](.workspace/0_doc/architecture/overview.md)）。

**対応 OS:** macOS / Linux / **Windows は WSL2（Ubuntu 26.04 LTS）**  
ネイティブ Windows での `cargo` はアプリケーション制御（os error 4551）の都合で推奨しません。

## 現状（x0014 / LAN 2-client presence）

`host-server` は **Rapier 60Hz**・Room Token **`sub` 別** Intent/Grab（最大 2）・**user_root entity 3/4**・**コライダー付き立方体 ×9（10..18）**・配信 **30Hz デルタ**。Grab は kinematic 追従（他 sub の箱は Steal しない）。  
`client` は `AUTH_USER=alice|bob` → 自 entity **3|4** をカメラ追従。他 user_root は色付きボックス。Desktop Grab / XR Grab / Noclip は従来どおり。  
**LAN:** zenohd が `0.0.0.0:7447` も listen。client/host は **`ZENOH_CONNECT`**（既定 `tcp/127.0.0.1:7447`）。ノートPC は PC の LAN IP へ。**alice を先に接続**（host first-come 割当とデモマップ一致）。  
Slice ID は `x` + 4桁16進（次は `x0015`）。規約は [`.workspace/roadmap/README.md`](.workspace/roadmap/README.md)。

| コンポーネント | パス |
|:---|:---|
| auth-server | [`auth-server/`](auth-server/) |
| world-server | [`world-server/`](world-server/) |
| assets-server | [`assets-server/`](assets-server/) |
| zenoh-router | [`zenoh-router/`](zenoh-router/) |
| protocol（共有ワイヤ契約） | [`protocol/`](protocol/) |
| client workspace | [`client/`](client/)（`app` + `host-server`） |

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
cargo run -p client

# 4) smoke（任意）
bash .scripts/smoke-assets.sh
bash .scripts/smoke-assets-glb.sh      # Coin GLB → CAS
bash .scripts/smoke-manifest-delta.sh  # Manifest + デルタ（zenohd 必須）
bash .scripts/smoke-rapier-host.sh     # Rapier 60Hz / net 30Hz（zenohd 必須）
bash .scripts/smoke-system-ui.sh       # system_ui unit + host_control コンパイル
bash .scripts/smoke-openxr.sh          # OpenXR モジュール unit + Desktop フォールバック compile
bash .scripts/smoke-desktop-noclip.sh  # Manifest Noclip + user_root（zenohd + auth/world 必須）
bash .scripts/smoke-lan-two-client.sh  # x0014 dual user_root unit + host banner
# 2 client（同マシン）: alice を先に、その後 bob
#   ALCHEMY_FORCE_DESKTOP=1 AUTH_USER=alice cargo run -p client
#   ALCHEMY_FORCE_DESKTOP=1 AUTH_USER=bob cargo run -p client
# LAN ノートPC: AUTH/WORLD/ASSETS_URL + ZENOH_CONNECT=tcp/<PC_IP>:7447 / AUTH_USER=bob
#   （PC 側 Docker は BIND_ALL=true。Windows ファイアウォールで 7447 と 4002–4004 を許可）
# ESC ダッシュ / XR Grab / Noclip は従来どおり。Desktop 強制: ALCHEMY_FORCE_DESKTOP=1
# OpenXR ローダー明示: ALCHEMY_OPENXR_LOADER=...
```

環境変数（任意）: `AUTH_URL` / `WORLD_URL` / `ASSETS_URL` / `ROOM_ID` / `AUTH_USER` / `AUTH_PASSWORD` / `ZENOH_CONNECT` / `ALCHEMY_BLOB_BLAKE3` / `ALCHEMY_HOST_PID` / `ALCHEMY_FORCE_DESKTOP` / `ALCHEMY_OPENXR_LOADER` / `STEAM_PATH`。

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
