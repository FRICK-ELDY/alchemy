# Alchemy

分散連合型 VR プラットフォーム（目標アーキテクチャ: [`.workspace/0_doc/architecture/overview.md`](.workspace/0_doc/architecture/overview.md)）。

**対応 OS:** macOS / Linux / **Windows（ネイティブ）**  
日常開発はネイティブ Windows の `cargo` で問題ない（かつて Smart App Control が ON だと os error 4551 で阻害されたが、現環境では OFF）。WSL2 は任意の代替。  
**テスター配布 ZIP**（ネイティブ Windows x64）は [Releases（Pre-release）](https://github.com/FRICK-ELDY/alchemy/releases) · 手順 [windows-tester-zip.md](.workspace/0_doc/manual/windows-tester-zip.md)。版名は `v0.0.0-dev.YYYYMMDDHHMM`（JST）。

## 現状（〜x007a）

手通し正本は **Desktop（x0020）**: 日常スタック（`:4002/:4003/:4004` + zenohd `:7447`）。手順は下記、env 例は [`client/env.desktop.example`](client/env.desktop.example)、機械検証は `make smoke-desktop-handthrough`。

`host-server` は **Rapier 60Hz**・Room Token **`sub` 別** Intent/Grab（Manifest user_root 本数・帯天井 4・`cnf.spk`）・**Presence / Leave / liveliness**・**30Hz 署名デルタ**・credential / Zenoh put worker。`client` は Home login・canonical **ManifestSeal**・**generation/revision** ゲート・decode 上限・Desktop/XR Grab。  
**暗号・門番（〜x0045）:** 検証失敗は discard、Intent rate + JWKS hard stale、Grab hold 距離クランプ。  
**運用・品質（x0046–x004d）:** adversarial E2E、launcher CI、asset placeholder + HTTP timeout、**localhost `/metrics`**、ClientIp 共有、**cargo-fuzz**、**Presence 容量 = Manifest entries**。  
**LAN / HTTPS / WAN:** zenohd + `ZENOH_CONNECT`；Tunnel（[`edge/cloudflared/`](edge/cloudflared/)）；WAN は **TLS + 短命 mTLS**（x0022 / [x0066](.workspace/roadmap/slices/x0066-ephemeral-mtls-world-issue.md) · [手順書](.workspace/0_doc/manual/relay-server-wan.md)）。world が CSR を署名し、zenohd は role CN ACL（[x0067](.workspace/roadmap/slices/x0067-zenohd-acl-put-family-host-guest.md)）。Relay Agent は互換フォールバック。  
**Assets:** CAS local / **R2**（GET は署名 URL 302）。PlaceRead / BlobGrant 案 B / CAS access peel は **[x006b](.workspace/roadmap/slices/x006b-place-read-capability.md)**–**[x006f](.workspace/roadmap/slices/x006f-cas-access-grant-peel.md)** **done**。**Hermetic E2E（x001f）:** `docker-compose.e2e.yml`。trust WAN 否定系 **[x0070](.workspace/roadmap/slices/x0070-headless-trust-wan-e2e.md)**、world RL + Room cap **[x0071](.workspace/roadmap/slices/x0071-world-rate-limit-room-cap.md)**、launcher discovery **[x0072](.workspace/roadmap/slices/x0072-launcher-discovery.md)** **done**。  
P4-1 描画統合は **[x0050](.workspace/roadmap/slices/x0050-xr-swapchain-wgpu.md)**–**[x0052](.workspace/roadmap/slices/x0052-xr-vk-loop-gpu-remove.md)** **done**。relay 契約は **[x0068](.workspace/roadmap/slices/x0068-relay-mtls-ttl-bound-ci.md)** **done**。部屋隔離（Room 鍵 AEAD）は **[x0069](.workspace/roadmap/slices/x0069-room-key-aead.md)** **done**。world Zenohex Adapter は **[x006a](.workspace/roadmap/slices/x006a-world-zenohex-adapter.md)** **done**。inventory ladder Wave 0–4（**[x0054](.workspace/roadmap/slices/x0054-inventory-grant-required-put.md)**–**[x0065](.workspace/roadmap/slices/x0065-origin-cert-thin.md)**）+ take-out/put-back UI **done**。体感 **[x0053](.workspace/roadmap/slices/x0053-xr-desktop-hotswitch.md)**（F8 / presence）**done**。  
P3 軽量衛生 **[x0073](.workspace/roadmap/slices/x0073-p3-hygiene-easy.md)** **done**。reserved reaper / orphan CAS GC **[x0074](.workspace/roadmap/slices/x0074-reserved-reaper-orphan-cas-gc.md)** **done**。BLAKE3 / seal-hash Port **[x0075](.workspace/roadmap/slices/x0075-blake3-seal-hash-port.md)** **done**。host/client 保証単位分割 **[x0076](.workspace/roadmap/slices/x0076-guarantee-unit-split.md)** **done**。bind 送信元分割 **[x0077](.workspace/roadmap/slices/x0077-bind-source-split.md)** **done**。mTLS temp ACL／掃除 **[x0078](.workspace/roadmap/slices/x0078-mtls-temp-acl-sweep.md)** **done**。DemoScene 共有テストベクタ **[x0079](.workspace/roadmap/slices/x0079-demoscene-shared-vector.md)** **done**。auth/world/relay metrics + `jti` 相関 **[x007a](.workspace/roadmap/slices/x007a-metrics-jti-correlation.md)** **done**。**次:** scene despawn。[roadmap](.workspace/roadmap/README.md) · [優先度バックログ](.workspace/roadmap/backlog-priority-2026-11-04.md)。

| コンポーネント | パス |
|:---|:---|
| auth-server | [`auth-server/`](auth-server/) |
| world-server | [`world-server/`](world-server/) |
| assets-server | [`assets-server/`](assets-server/) |
| relay-server | [`relay-server/`](relay-server/)（zenohd 設定・待受） |
| relay-agent | [`relay-agent/`](relay-agent/)（Allocation → 短命 mTLS） |
| protocol（共有ワイヤ契約） | [`protocol/`](protocol/) |
| client workspace | [`client/`](client/)（`app` + `host-server`） |
| edge（Cloudflare Tunnel） | [`edge/cloudflared/`](edge/cloudflared/) |

ルートに Cargo workspace は置きません。Rust のビルド根は `client/`、契約クレートは独立した `protocol/` です。

## Windows（ネイティブ）

1. [rustup](https://rustup.rs/) で Rust stable
2. Docker Desktop（auth / world / assets）
3. `cargo install zenohd`（または同等の zenohd）
4. XR 手通しは Windows + SteamVR / OpenXR（WSL は実 HMD 非保証）

PowerShell から `make` / `bash .scripts/*.sh` を使う場合は Git Bash 等の bash が必要。Elixir 系 CI は Docker 既定のためホストの `mix` は不要。

### 任意: WSL2（Ubuntu 26.04 LTS）

Linux 寄りのシェル／CI 再現が欲しいときの代替。主軸ではない。

```powershell
wsl --install -d Ubuntu-26.04
wsl -d Ubuntu-26.04
```

WSL 内で `bash .scripts/wsl-setup.sh`（build-essential / wgpu 用ライブラリ / rustup / zenohd）。Docker Desktop の **WSL Integration** で当該ディストロを有効化。リポジトリは WSL 側パス（`\\wsl$\...`）の方が I/O が速い。

## 前提ツール（macOS / Linux / Windows）

| ツール | 用途 | 備考 |
|:---|:---|:---|
| Rust (stable) | client / host-server / protocol | [rustup](https://rustup.rs/)（任意で `wsl-setup.sh`） |
| Docker / Docker Compose | auth / world / assets | Desktop、または Linux ネイティブ |
| zenohd | ローカルリレー（relay-server） | `cargo install zenohd` |

任意: Elixir 1.19 / OTP 28（ホスト直実行したい場合のみ）。

Linux（および任意の WSL）で `client` をビルドする場合の例（`wsl-setup.sh` に含む）:

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
zenohd -c relay-server/config.json5

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
bash .scripts/smoke-zenoh-wan.sh        # x001b/x0022 WAN: ZENOH_CONNECT=tls/… 必須
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
# Zenoh WAN TLS+mTLS（x0022 / x004e / x0066 · ConoHa）:
#   初回・正本: .workspace/0_doc/manual/relay-server-wan.md（DNS/LE: relay-server-dns.md）
#   設定の載せ替え: 下記「VPS 上の zenohd 設定更新」
#   export ZENOH_CONNECT=tls/<VPS_HOST_or_DNS>:7447
#   拠点 host + 外 client（x001a URL）で同じ ROOM_ID。smoke: bash .scripts/smoke-zenoh-wan.sh
# ESC ダッシュ / Home タブ / XR Grab / Noclip。Desktop 強制: ALCHEMY_FORCE_DESKTOP=1
# OpenXR ローダー明示: ALCHEMY_OPENXR_LOADER=...
# XR SceneGpu（x0052 既定）: FORCE_DESKTOP なし・SteamVR
```

### VPS 上の zenohd 設定更新

正本は常にリポジトリの [`relay-server/config.wan.json5`](relay-server/config.wan.json5)。VPS の `/etc/alchemy/zenoh-wan.json5` へコピーしたあと、**listen 証明書パスだけ**運用中の値に手で戻す（リポジトリ正本は例の `server-*.pem`）。`enable_mtls` / `root_ca_certificate` / `access_control` / `shared_memory` はリポジトリ側を優先する。経路 B のパス例は [relay-server-dns.md](.workspace/0_doc/manual/relay-server-dns.md)（certbot `live/…` または copy 先の `server-*.pem`）。

手元（PowerShell）:

```powershell
scp .\relay-server\config.wan.json5 account@<VPS_IP>:/tmp/zenoh-wan.json5
```

VPS:

```bash
# いま動いている listen パスを控える
grep -E 'listen_private_key|listen_certificate' /etc/alchemy/zenoh-wan.json5
sudo cp /etc/alchemy/zenoh-wan.json5 /etc/alchemy/zenoh-wan.json5.bak
sudo cp /tmp/zenoh-wan.json5 /etc/alchemy/zenoh-wan.json5
# 控えた listen_* をエディタで戻す（経路 A: 正本の server-*.pem のままなら不要）
# sudo nano /etc/alchemy/zenoh-wan.json5
# x0067 正本: interest egress 欠落（388b768）や room/** put 統合をここで落とす
grep -E 'listen_|root_ca|shared_memory|enabled:|host-put-sim|host-egress-declare|guest-egress-interest|guest-egress-sim|guest-put-input|guest-sub-sim' /etc/alchemy/zenoh-wan.json5
# 期待: 上に host-egress-declare / guest-egress-interest / host-put-sim あり
# 旧統合・旧 id が残っていたら失敗（コメント中の "room/**" 文言には反応しない）
grep -E 'host-room|guest-room|key_exprs: \["alchemy/dev/room/\*\*"\]' /etc/alchemy/zenoh-wan.json5 && echo 'FAIL: legacy room/** ACL still present' || echo 'OK: no legacy room/** key_exprs'
sudo systemctl restart alchemy-zenohd
systemctl is-active alchemy-zenohd
```

再起動後は既存の host / client の Zenoh セッションが切れる。**host を起動し直して置きっぱなし**にしてから client を繋ぐ。接続例: `ZENOH_CONNECT=tls/relay.example.tld:7447`（詳細・初回セットアップは [relay-server-wan.md](.workspace/0_doc/manual/relay-server-wan.md)）。

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
make ci-launcher   # x0047/x0073 alchemy-launcher fmt/clippy/deny/build/test
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
| `launcher` | `launcher/` で fmt / clippy / deny / build / test（x0047 / x0073） |

## ライセンス

Alchemy は責務境界に沿った **4階層マルチライセンス**です。詳細は [`LICENSE`](LICENSE) と [`TRADEMARKS.md`](TRADEMARKS.md) を参照してください。

| 階層 | 対象（現行パス） | SPDX | 公開形態 |
|:---|:---|:---|:---|
| ① ワイヤ契約・中継規格 | [`protocol/`](protocol/)、[`relay-server/`](relay-server/)、[`relay-agent/`](relay-agent/) | `Apache-2.0` | 完全 OSS |
| ② 必須コア・クライアント | [`auth-server/`](auth-server/)、[`assets-server/`](assets-server/)、[`world-server/`](world-server/)、[`client/`](client/)（`host-server` / `shared-sim` 含む）、[`launcher/`](launcher/)、[`shared/alchemy_auth_client/`](shared/alchemy_auth_client/)、[`shared/alchemy_client_ip/`](shared/alchemy_client_ip/)、[`shared/alchemy_rate_limit/`](shared/alchemy_rate_limit/) | `MPL-2.0` | 完全 OSS |
| ③ 検疫・監査アルゴリズム | pgvector 検疫ワーカー、Spot-Check 詳細ヒューリスティック（別リポジトリ） | Proprietary | 非公開（OSS 側は Behaviour のみ） |
| ④ 決済・商用マーケット | `market-server` / `BillingProvider`（未導入） | `BSL-1.1` | Source-Available（商用競合制限） |

「VRAlchemy」「AlchemyEngine」および公式ロゴ・公認バッジはオープンソース許諾の対象外です（[`TRADEMARKS.md`](TRADEMARKS.md)）。
