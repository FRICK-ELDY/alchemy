# Roadmap

## Slice A — Local Zenoh 縦スライス（done）

目標: ローカル `zenohd` + `host-server` Publish + `client` Subscribe 描画 + `auth-server` JWT/JWKS。  
**OS:** macOS / Linux / Windows は **WSL2（Ubuntu 26.04 LTS）**（Docker + Rust）

| Phase | 内容 | 状態 |
|:---|:---|:---|
| 0 | `auth-server` を alchemy-engine から移植・Docker 起動 | done |
| 1 | `zenoh-router` 設定（ローカル zenohd） | done |
| 2 | `protocol/` 最小 RenderFrame + PROTOCOL_PIN | done |
| 3 | `client/host-server` 30Hz Publish | done |
| 4 | `client/app` Subscribe + wgpu 最小描画 | done |
| — | Cargo をルートから `client/` へ移設、`protocol/` を共有契約として独立 | done |

### Slice A で保証する / しない

| 保証する | 保証しない |
|:---|:---|
| 同一マシン上で `zenohd` → Publish → Subscribe → 最小描画が通る | Room Token / 入室ゲート |
| `auth-server` の JWT/JWKS 起動（Docker） | assets / LOD / Quota |
| `PROTOCOL_PIN` 付き `RenderFrame` の encode/decode | デルタ圧縮・Rapier・Formula |
| キー式 `alchemy/dev/room/local/frame` | 公式リレー・P2P・verifier・launcher |

### Slice A 起動手順

```bash
# macOS / Linux / WSL2 (Ubuntu-26.04)
cd auth-server && docker compose up -d && cd ..
zenohd -c zenoh-router/config.json5
cd client
cargo run -p host-server   # 別ターミナル
cargo run -p client        # 別ターミナル
```

キー式: `alchemy/dev/room/local/frame`

---

## Slice B — Room Token 門番（done）

目標: 既存 Zenoh 縦スライスに **短命 Room Token** を載せ、host-server が不正入力を drop する。

| Phase | 内容 | 状態 |
|:---|:---|:---|
| 0 | `world-server` 最小（Room 作成・Room Token 発行・JWKS） | done |
| 1 | `protocol/` `InputEvent` + `room_token` + PROTOCOL_PIN bump | done |
| 2 | `host-server` 入力 Subscribe + Token 検証失敗は drop | done |
| 3 | `client` Token 取得 + InputEvent Publish | done |

### Slice B で保証する / しない

| 保証する | 保証しない |
|:---|:---|
| Room 作成と短命 Room Token（RS256）の発行 | 5段階公開範囲（Private/LAN/Friends/…） |
| `/.well-known/jwks.json` による Room Token 検証用鍵配信 | verifier / Spot-Check / ホスト移譲 |
| 入力ワイヤに `room_token` を載せ、host が検証 | S2S 連合・World カタログ永続化 |
| 検証失敗入力の drop | assets / billing / market / P2P |

### Slice B 起動手順

```bash
cd auth-server && docker compose up -d && cd ..
cd world-server && docker compose up -d && cd ..
zenohd -c zenoh-router/config.json5
cd client
cargo run -p host-server   # 別ターミナル（WORLD_URL 既定 http://127.0.0.1:4003）
cargo run -p client
```

キー式:

- frame: `alchemy/dev/room/local/frame`
- input: `alchemy/dev/room/local/input`

### Slice B でやらない（後続 / Slice C+）

- assets-server / アセット LOD / Quota
- 公式リレー（ConoHa）・サードパーティリレー
- P2P・verifier・launcher 本実装
- Rapier / Formula / デルタ圧縮
- client の git submodule 化
- Friendship / グループ / 公開範囲 ACL

---

## Slice C — assets 最小（未着手）

描画に実アセットが必要になったら着手。フル ER は書かず、先に保証境界とオブジェクトモデル（[assets-server.md](0_doc/architecture/assets-server.md)）を正本とする。

---

## ローカル CI

```bash
make ci
make ci-rust
make ci-protocol
make ci-auth
make ci-world
```

詳細は [README.md](../README.md)。
