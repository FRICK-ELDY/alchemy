# world-server

> **正本コード**: [`world-server/`](../../../world-server/)  
> **ステータス**: Slice B（Room Token 最小）

## 図上の位置

[overview.md](overview.md) の運営側コア — 案内所・門番。Slice B では **Room Token 発行のみ** を実装する。

## 保証すること / しないこと（Slice B）

| 保証する | 保証しない |
|:---|:---|
| Room の作成（プロセス内・揮発） | World 一覧の永続 DB / Ash |
| 短命 Room Token（RS256）の発行 | 5段階公開範囲・Friendship ACL |
| `GET /.well-known/jwks.json` | verifier / ホスト死活監視 / S2S 連合 |
| 開発用匿名トークン発行（`WORLD_ALLOW_ANON_TOKENS`） | alchemy:// ディープリンク本実装 |

## 契約（Room Token）

| Claim | 内容 |
|:---|:---|
| `sub` | 利用者 ID（匿名時は `anon`） |
| `room_id` | 部屋 ID（ASCII、`/` なし） |
| `role` | `host` \| `guest` |
| `iss` | `alchemy-world` |
| `aud` | `alchemy-room` |
| `exp` | 短命（既定 300 秒） |
| `jti` | UUID |

検証は host-server が JWKS を取得して行う（転送経路は Zenoh、権威は Token）。

## API（Slice B）

| Method | Path | 内容 |
|:---|:---|:---|
| GET | `/health` | 生存確認 |
| GET | `/.well-known/jwks.json` | Room Token 検証用公開鍵 |
| POST | `/api/v1/rooms` | Room 作成 `{ "room_id"?: string }` |
| POST | `/api/v1/rooms/:room_id/tokens` | Token 発行 `{ "role": "host"\|"guest", "sub"?: string }` |

## 起動（Docker）

```bash
cd world-server
docker compose up -d
# http://localhost:4003/health
# http://localhost:4003/.well-known/jwks.json
```

## 関連

- ワイヤ: [protocol.md](protocol.md)（`InputEvent.room_token`）
- 転送: [zenoh-router.md](zenoh-router.md)
- 認証ユーザー JWT: [auth-server.md](auth-server.md)（Slice B では Room Token 発行に未接続可）
