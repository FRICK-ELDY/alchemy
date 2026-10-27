# world-server

> **正本コード**: [`world-server/`](../../../world-server/)  
> **ステータス**: x0027（DemoScene）+ x0026（ManifestSeal）+ x0025（HostAttestation）+ Slice B.5

## 図上の位置

[overview.md](overview.md) の運営側コア — 案内所・門番。Slice B では **Room Token 発行**、Slice B.5 で **auth-server ユーザー JWT 検証** を追加。

## 保証すること / しないこと（Slice B.5）

| 保証する | 保証しない |
|:---|:---|
| Room の作成（プロセス内・揮発） | World 一覧の永続 DB / Ash |
| 短命 Room Token（RS256）の発行 | 5段階公開範囲・Friendship ACL |
| `GET /.well-known/jwks.json`（Room Token 用） | verifier / ホスト死活監視 / S2S 連合 |
| Room API は auth-server Bearer JWT 必須（`sub` を Room Token に転写） | 匿名 Token（既定 OFF。`WORLD_ALLOW_ANON_TOKENS=true` のみ） |
| auth JWKS（`AUTH_JWKS_URL`）によるユーザー JWT 検証 | auth の revocation 再確認 |
| HostAttestation JWT（`typ=host_attestation`）を host Room Token のみ発行（x0025）。発行失敗は 5xx（token 検証失敗は 401） | Room DB 永続・UGC 編集 |
| ManifestSeal JWT（`typ=manifest_seal` / `aud=alchemy-manifest-seal`）を host Room Token のみ発行（x0026）。body は blake3 のみ受け取る | seal 時の entries 改変（host が body を組む） |
| DemoScene（entries + loco）を Room 作成時に紐付け。`GET …/scene` は **host** Room Token のみ（x0027） | guest への scene 配布・複数シーン切替 UI |

## 契約（Room Token）

| Claim | 内容 |
|:---|:---|
| `sub` | auth ユーザー ID（UUID）。匿名時のみ `anon` 等 |
| `room_id` | 部屋 ID（ASCII、`/` なし） |
| `role` | `host` \| `guest` |
| `iss` | `alchemy-world` |
| `aud` | `alchemy-room` |
| `exp` | 短命（既定 300 秒） |
| `jti` | UUID |

検証は host-server が JWKS を取得して行う（転送経路は Zenoh、権威は Token）。

## API（Slice B.5）

| Method | Path | Auth | 内容 |
|:---|:---|:---|:---|
| GET | `/health` | 不要 | 生存確認 |
| GET | `/.well-known/jwks.json` | 不要 | Room Token 検証用公開鍵 |
| POST | `/api/v1/rooms` | Bearer（auth JWT） | Room 作成 |
| POST | `/api/v1/rooms/:room_id/tokens` | Bearer（auth JWT） | Token 発行（`sub` は JWT から） |
| POST | `/api/v1/rooms/:room_id/host_keys` | Bearer（**host Room Token**） | HostAttestation 発行（`host_pub` base64url・x0025） |
| POST | `/api/v1/rooms/:room_id/manifest_seal` | Bearer（**host Room Token**） | ManifestSeal 発行（`revision` + `manifest_blake3`・x0026） |
| GET | `/api/v1/rooms/:room_id/scene` | Bearer（**host Room Token**） | DemoScene `{ revision, entries, locomotion }`（x0027） |

## 起動（Docker）

auth-server を先に起動すること（JWKS 参照）。

```bash
cd auth-server && docker compose up -d && cd ..
cd world-server && docker compose up -d
# http://localhost:4003/health
# AUTH_JWKS_URL 既定: http://host.docker.internal:4002/.well-known/jwks.json
```

## 関連

- ワイヤ: [protocol.md](protocol.md)（`InputEvent.room_token`）
- 転送: [zenoh-router.md](zenoh-router.md)
- 認証ユーザー JWT: [auth-server.md](auth-server.md)
