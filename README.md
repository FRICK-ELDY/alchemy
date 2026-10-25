# Alchemy

分散連合型 VR プラットフォーム（目標アーキテクチャ: [`.workspace/0_doc/architecture/overview.md`](.workspace/0_doc/architecture/overview.md)）。

**対応 OS:** macOS / Linux / **Windows は WSL2（Ubuntu 26.04 LTS）**  
ネイティブ Windows での `cargo` はアプリケーション制御（os error 4551）の都合で推奨しません。

## 現状（x0013 / Desktop Grab）

`host-server` は **Rapier 60Hz**・**`is_finite` 入力ゲート**・配信 **30Hz デルタ**に加え、`SceneManifest.locomotion` 既定 **Noclip** で **user_root** を積分し、**コライダー付き立方体 ×9（entity 10..18）** を SoA `grabbable` 付きで置く。Grab 中は **kinematic 追従**、離すと dynamic に戻す。  
`client`（Desktop）は **WASD / Space / C** → Intent（**マウスルック yaw 相対**）。ダッシュ閉でクリックすると **Confined**（失敗時 Locked）。look 中は箱を俯瞰して明るくなったら **LMB/RMB/E** で **Grab**。OpenXR は **左右 thumbstick（aim 相対）** → 同じ Intent。カメラ位置は user_root。  
**Menu**／ESC ダッシュは個人 **DashRoot**（Esc で look／Grab 解放）。ダッシュ閉時 XR は aim **ワールドレーザー**＋ **Grab hold**。実機 XR 正本は **Windows + SteamVR**（WSL は HMD 非保証）。  
Slice ID は `x` + 4桁16進（次は `x0014`）。規約は [`.workspace/roadmap/README.md`](.workspace/roadmap/README.md)。

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
# ESC ダッシュ: client 起動後 Esc。ボタンにマウスを当てると色が変わる。WASD+Space+C で Noclip（マウスルックは後続）。
# XR: SteamVR 2.12+ + Menu。レーザー UI hover。Grab hold（squeeze）。親指スティック Noclip。
# client+host 終了は ALCHEMY_HOST_PID か host_control。Desktop 強制: ALCHEMY_FORCE_DESKTOP=1
# OpenXR ローダー明示（Steam が特殊パスのとき）: ALCHEMY_OPENXR_LOADER=D:\...\openxr_loader.dll
```

環境変数（任意）: `AUTH_URL` / `WORLD_URL` / `ASSETS_URL` / `ROOM_ID` / `AUTH_USER` / `AUTH_PASSWORD` / `ALCHEMY_BLOB_BLAKE3` / `ALCHEMY_HOST_PID` / `ALCHEMY_FORCE_DESKTOP` / `ALCHEMY_OPENXR_LOADER` / `STEAM_PATH`。

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
