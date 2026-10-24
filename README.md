# Alchemy

分散連合型 VR プラットフォーム（目標アーキテクチャ: [`.workspace/0_doc/architecture/overview.md`](.workspace/0_doc/architecture/overview.md)）。

**対応 OS:** macOS / Linux / **Windows は WSL2（Ubuntu 26.04 LTS）**  
ネイティブ Windows での `cargo` はアプリケーション制御（os error 4551）の都合で推奨しません。

## 現状（Slice B）

ローカル `zenohd` → `host-server` が RenderFrame を Publish → `client` が Subscribe して表示。  
入力は Room Token 付き `InputEvent`。`world-server` が Token / JWKS を発行。認証ユーザー JWT は `auth-server`。

| コンポーネント | パス |
|:---|:---|
| auth-server | [`auth-server/`](auth-server/) |
| world-server | [`world-server/`](world-server/) |
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
| Docker / Docker Compose | auth-server / world-server | Desktop + WSL Integration、または Linux ネイティブ |
| zenohd | ローカルルーター | `cargo install zenohd` |

任意: Elixir 1.19 / OTP 28（ホスト直実行したい場合のみ）。

Linux / WSL で `client` をビルドする場合の例（`wsl-setup.sh` に含む）:

```bash
sudo apt-get install -y pkg-config libxkbcommon-dev libwayland-dev \
  libx11-dev libxcursor-dev libxi-dev libxrandr-dev
```

## Slice B 起動（macOS / Linux / WSL）

```bash
# 1) auth + world
cd auth-server && docker compose up -d && cd ..
cd world-server && docker compose up -d && cd ..

# 2) zenohd
zenohd -c zenoh-router/config.json5

# 3) host + client（別ターミナル）
cd client
cargo run -p host-server
cargo run -p client
```

環境変数（任意）: `WORLD_URL`（既定 `http://127.0.0.1:4003`）、`ROOM_ID`（既定 `local`）、`ALCHEMY_ROOM_TOKEN`（指定時は発行 API をスキップ）。

詳細・キー式は [`.workspace/roadmap.md`](.workspace/roadmap.md)。

## ローカル CI

GitHub Actions と同じゲートを手元で実行します。**auth / world は全 OS で Docker 既定**。

```bash
chmod +x .scripts/ci.sh   # 初回のみ
make ci
make ci-rust
make ci-protocol
make ci-auth
make ci-world
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
