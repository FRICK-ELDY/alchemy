# world-server

> **正本コード**: [`world-server/`](../../../world-server/)  
> **ステータス**: x0028（Room 永続 Ash/SQLite）+ x0027 / x002c（DemoScene + 初期 TRS）+ x0026 seal + x0025 attestation

## 図上の位置

[overview.md](overview.md) の運営側コア — 案内所・門番。Slice B では **Room Token 発行**、Slice B.5 で **auth-server ユーザー JWT 検証** を追加。

## 保証すること / しないこと（Slice B.5）

| 保証する | 保証しない |
|:---|:---|
| Room の作成（Ash + SQLite。`owner_sub` + DemoScene + `visibility`・x0028/x0034）。**per-owner 件数上限**（既定 32）+ **作成レート制限**（既定 10/60s・**insert 前消費**・429 `Retry-After`・x004c） | World 一覧 UI / Group；厳密な並行 cap；削除履歴の課金；成功時のみの rate 課金 |
| 短命 Room Token（RS256）の発行（**`cnf.spk` = session_pub・x003e**） | Friendship 正本（auth）。Token 前に S2S 照会；`jkt` 互換 |
| 7段階公開範囲 ACL（Token 発行時・x0034–x0036） | unfollow / visibility 変更時の既発行 Token 即時失効（TTL まで有効） |
| client Home「新規ワールド作成」仮 UI でアクセスレベル選択→ create/PUT + 入室（x0037） | UI デザイン最終化；`room_id` 動的切替；テンプレ複数；入室中の途中変更 UI |
| `GET /.well-known/jwks.json`（Room Token 用） | verifier / ホスト死活監視 / S2S 連合 |
| Room API は auth-server Bearer JWT 必須（`sub` を Room Token に転写） | 匿名 Token（既定 OFF。`WORLD_ALLOW_ANON_TOKENS=true` のみ） |
| auth JWKS（`AUTH_JWKS_URL`）によるユーザー JWT 検証 | auth の revocation 再確認 |
| HostAttestation JWT（`typ=host_attestation` + **`host_generation`**）を host Room Token のみ発行（x0025 / **x003a**）。同一 `host_pub` は generation 維持、鍵変更で bump。発行失敗は 5xx（token 検証失敗は 401） | UGC scene 編集；完全な handoff UX |
| ManifestSeal JWT（`typ=manifest_seal` / `aud=alchemy-manifest-seal` + **`host_generation`**）を host Room Token のみ発行（x0026 / **x0039** / **x003a**）。**DB scene + HostAttestation から canonical blake3 を再計算**し、host 提出 hash と不一致なら 422（署名 oracle 廃止）。`manifest-seal-hash` CLI 必須 | UGC scene 編集；host ローカル改変 body への seal |
| DemoScene（entries + loco + **初期 quat/scale**）を Room 作成時に紐付け。`GET …/scene` は **host** Room Token のみ（x0027 / x002c） | guest への scene 配布・複数シーン切替 UI・collider scale |

## 契約（Room Token）

| Claim | 内容 |
|:---|:---|
| `sub` | auth ユーザー ID（UUID）。匿名時のみ `anon` 等 |
| `room_id` | 部屋 ID（ASCII、`/` なし） |
| `role` | `host` \| `guest`（→ sim/HTTP capability 表は [protocol.md](protocol.md) x002a） |
| `iss` | `alchemy-world` |
| `aud` | `alchemy-room` |
| `exp` | 短命（既定 300 秒） |
| `jti` | UUID |
| `cnf.spk` | Intent 用 Ed25519 `session_pub`（base64url・32B・x003e）。`POST …/tokens` で必須 |

検証は host-server が JWKS を取得して行う（転送経路は Zenoh、権威は Token）。bind 時は Intent の `session_pub` と `cnf.spk` の一致が必須（盗聴 JWT だけでは別鍵 bind 不可）。HTTP credential（scene / attest / seal）は `role` のみ見る。

## API（Slice B.5）

### 公開範囲（x0033–x0036）

| 値 | 入場 |
|:---|:---|
| `lan` | LAN 上のみ。Tunnel / 公網 IP は拒否（x0036） |
| `private` | owner のみ |
| `friends` | 確定フレンド（auth S2S） |
| `friends_plus` | Friends ∪ FoF |
| `followers` | owner のフォロワー |
| `followers_plus` | Followers ∪ follow-of-follower |
| `public` | 認証済みなら誰でも（Room 既定・デモ互換） |

| Method | Path | Auth | 内容 |
|:---|:---|:---|:---|
| GET | `/health` | 不要 | 生存確認 |
| GET | `/.well-known/jwks.json` | 不要 | Room Token 検証用公開鍵 |
| POST | `/api/v1/rooms` | Bearer（auth JWT） | Room 作成（任意 `visibility`）。超過は `owner_room_limit` 403 / `rate_limited` 429（x004c） |
| PUT | `/api/v1/rooms/:room_id/visibility` | Bearer（auth JWT） | owner のみ更新 |
| POST | `/api/v1/rooms/:room_id/tokens` | Bearer（auth JWT） | Token 発行（visibility ACL 後） |
| POST | `/api/v1/rooms/:room_id/host_keys` | Bearer（**host Room Token**） | HostAttestation 発行（`host_pub` base64url・`host_generation`・x0025 / x003a） |
| POST | `/api/v1/rooms/:room_id/manifest_seal` | Bearer（**host Room Token**） | ManifestSeal 発行（`revision` + `host_generation` + `manifest_blake3` + `host_attestation`；canonical 照合・x0039 / x003a） |
| GET | `/api/v1/rooms/:room_id/scene` | Bearer（**host Room Token**） | DemoScene `{ revision, entries, locomotion }`（x0027） |

## 永続（x0028）

| 項目 | 内容 |
|:---|:---|
| 既定 | AshSqlite。DB ファイルは `priv/data/world_dev.db`（test は `world_test.db`） |
| 上書き | env `WORLD_DATABASE_PATH`（**`MIX_ENV=test` では無視**・x003b；assets CAS 隔離と同型） |
| スキーマ | `rooms(room_id PK, owner_sub, scene JSON, visibility, timestamps)` |
| Postgres 切替 | 属性は Postgres 互換のみ。運営規模拡大時は `ash_postgres` + `AshPostgres.DataLayer` / `AshPostgres.Repo` に差し替え、`mix ash.codegen` で再生成。API facade（`World.Rooms`）は変えない |
| 障害 | SQLite / Ash 失敗は `:storage_error` → HTTP 503 `storage_unavailable`（404/422 に潰さない） |
| prod volume | `world_prod_keys` に JWT PEM と SQLite を同居（`/var/lib/world`）。`-v` で両方消える |

## 起動（Docker）

auth-server を先に起動すること（JWKS 参照）。

```bash
cd auth-server && docker compose up -d && cd ..
cd world-server && docker compose up -d
# http://localhost:4003/health
# AUTH_JWKS_URL 既定: http://host.docker.internal:4002/.well-known/jwks.json
# mix ash.setup が SQLite migrate を実行
```

**prod（x001e / x003b）:** `docker-compose.prod.yml` + `.env.prod`（`.env.prod.example` 参照）。  
`AUTH_INTERNAL_TOKEN`（auth/world 一致）と world の `AUTH_INTERNAL_URL` が必須。検証: `make smoke-prod-compose`（config）/ `SMOKE_PROD_UP=1 make smoke-prod-compose`（`/health`）。

## 関連

- ワイヤ: [protocol.md](protocol.md)（`InputEvent.room_token`）
- 転送: [zenoh-router.md](zenoh-router.md)
- 認証ユーザー JWT: [auth-server.md](auth-server.md)
