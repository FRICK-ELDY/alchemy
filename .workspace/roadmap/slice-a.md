# Slice A — Local Zenoh 縦スライス（done）

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

## 保証する / しない

| 保証する | 保証しない |
|:---|:---|
| 同一マシン上で `zenohd` → Publish → Subscribe → 最小描画が通る | Room Token / 入室ゲート |
| `auth-server` の JWT/JWKS 起動（Docker） | assets / LOD / Quota |
| `PROTOCOL_PIN` 付き `RenderFrame` の encode/decode | デルタ圧縮・Rapier・Formula |
| キー式 `alchemy/dev/room/local/frame` | 公式リレー・P2P・verifier・launcher |

## 起動手順

```bash
# macOS / Linux / WSL2 (Ubuntu-26.04)
cd auth-server && docker compose up -d && cd ..
zenohd -c zenoh-router/config.json5
cd client
cargo run -p host-server   # 別ターミナル
cargo run -p client        # 別ターミナル
```

キー式: `alchemy/dev/room/local/frame`

← [目次](README.md)
