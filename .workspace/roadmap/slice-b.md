# Slice B — Room Token 門番（done）

目標: 既存 Zenoh 縦スライスに **短命 Room Token** を載せ、host-server が不正入力を drop する。

| Phase | 内容 | 状態 |
|:---|:---|:---|
| 0 | `world-server` 最小（Room 作成・Room Token 発行・JWKS） | done |
| 1 | `protocol/` `InputEvent` + `room_token` + PROTOCOL_PIN bump | done |
| 2 | `host-server` 入力 Subscribe + Token 検証失敗は drop | done |
| 3 | `client` Token 取得 + InputEvent Publish | done |
| 4 | auth JWT 必須化（匿名 Token 既定 OFF）— Slice B.5 | done |

## 保証する / しない

| 保証する | 保証しない |
|:---|:---|
| Room 作成と短命 Room Token（RS256）の発行 | 5段階公開範囲（Private/LAN/Friends/…） |
| `/.well-known/jwks.json` による Room Token 検証用鍵配信 | verifier / Spot-Check / ホスト移譲 |
| 入力ワイヤに `room_token` を載せ、host が検証 | S2S 連合・World カタログ永続化 |
| 検証失敗入力の drop | assets / billing / market / P2P |
| Room API は auth-server Bearer JWT 必須（`sub` 転写） | 匿名 Token（`WORLD_ALLOW_ANON_TOKENS` 明示時のみ） |

## 起動手順

```bash
cd auth-server && docker compose up -d && cd ..
cd world-server && docker compose up -d && cd ..
zenohd -c zenoh-router/config.json5
cd client
cargo run -p host-server   # auth login (alice) → Room Token
cargo run -p client
```

キー式:

- frame: `alchemy/dev/room/local/frame`
- input: `alchemy/dev/room/local/input`

環境変数（任意）: `AUTH_URL` / `WORLD_URL` / `AUTH_USER` / `AUTH_PASSWORD` / `ALCHEMY_AUTH_TOKEN` / `ALCHEMY_ROOM_TOKEN`

## やらない（後続）

- assets-server / アセット LOD / Quota
- 公式リレー（ConoHa）・サードパーティリレー
- P2P・verifier・launcher 本実装
- Rapier / Formula / デルタ圧縮
- client の git submodule 化
- Friendship / グループ / 公開範囲 ACL

正本: [world-server.md](../0_doc/architecture/world-server.md)

← [目次](README.md)
