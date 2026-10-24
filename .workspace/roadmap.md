# Roadmap

## Slice A — Local Zenoh 縦スライス（進行中）

目標: ローカル `zenohd` + `host-server` Publish + `client` Subscribe 描画 + `auth-server` JWT/JWKS。

| Phase | 内容 | 状態 |
|:---|:---|:---|
| 0 | `auth-server` を alchemy-engine から移植・Docker 起動 | done |
| 1 | `zenoh-router` 設定（ローカル zenohd） | done |
| 2 | `crates/protocol` 最小 RenderFrame + PROTOCOL_PIN | done |
| 3 | `crates/host-server` 30Hz Publish | done |
| 4 | `crates/client` Subscribe + wgpu 最小描画 | done |

### Slice A でやらない（後続）

- assets-server / アセット LOD
- 公式リレー（ConoHa）・サードパーティリレー
- Room Token（world-server）
- P2P・verifier・launcher 本実装
- Rapier / Formula / デルタ圧縮

### 起動手順（Slice A）

```powershell
# 1) auth
cd D:\github\frick\alchemy\auth-server
docker compose up -d

# 2) zenohd
cd D:\github\frick\alchemy\zenoh-router
zenohd -c config.json5

# 3) host + client（別ターミナル）
cd D:\github\frick\alchemy
cargo run -p host-server
cargo run -p client
```

キー式: `alchemy/dev/room/local/frame`
