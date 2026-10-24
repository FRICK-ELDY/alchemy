# Roadmap

## Slice A — Local Zenoh 縦スライス（進行中）

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

### Slice A でやらない（後続）

- assets-server / アセット LOD
- 公式リレー（ConoHa）・サードパーティリレー
- Room Token（world-server）
- P2P・verifier・launcher 本実装
- Rapier / Formula / デルタ圧縮
- client の git submodule 化

### 起動手順（Slice A）

```bash
# macOS / Linux / WSL2 (Ubuntu-26.04)
# Windows: wsl -d Ubuntu-26.04 のあと、必要なら bash .scripts/wsl-setup.sh
cd auth-server && docker compose up -d && cd ..
zenohd -c zenoh-router/config.json5
cd client
cargo run -p host-server   # 別ターミナル
cargo run -p client        # 別ターミナル
```

キー式: `alchemy/dev/room/local/frame`

### ローカル CI

```bash
make ci
make ci-rust
```

詳細は [README.md](../README.md)。
