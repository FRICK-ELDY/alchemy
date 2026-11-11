# world-server

> **正本コード**: [`world-server/`](../../../world-server/)  
> **ステータス**: x0028（Room 永続 Ash/SQLite）+ x0027 / x002c（DemoScene + 初期 TRS）+ x0026 seal + x0025 attestation

## 図上の位置

[overview.md](overview.md) の運営側コア — 案内所・門番。Slice B では **Room Token 発行**、Slice B.5 で **auth-server ユーザー JWT 検証** を追加。

## 保証すること / しないこと（Slice B.5）

| 保証する | 保証しない |
|:---|:---|
| Room の作成（Ash + SQLite。`owner_sub` + DemoScene + `visibility`・x0028/x0034） | Group ACL |
| `GET /api/v1/rooms` カタログ（見える Room のみ・秘密なし・x007c）。**x0080**: 非 mine は `state=active`+未期限切れ lease のみ；`?mine=1` は dormant 定義も可。要約に `state` / `active_host_sub` | Frame 到達保証；`room_key` / `wire_cap` の一覧同梱 |
| `owner_sub`（永続）と `active_host_sub`（計算ホスト）の分離。`POST …/handover` / `POST …/takeover`（CAS）/ host lease 失効で昇格・休止。Host Token は active_host または dormant からの owner 再開に限定（x0080） | 偽 Zenoh Notice 単独での Host Token；pose/grab のシームレス継承；Room 行の物理 DELETE |
| 短命 Room Token（RS256）の発行（**`cnf.spk` = session_pub・x003e**） | Friendship 正本（auth）。Token 前に S2S 照会；`jkt` 互換 |
| 7段階公開範囲 ACL（Token 発行時・x0034–x0036） | unfollow 単独の既発行 Token 即時失効（TTL まで有効） |
| 短命 mTLS（x0066）: Token 発行時の任意 `csr_pem` → **CSR Subject 無視**・role CN（`alchemy-host` / `alchemy-guest`）で client cert 署名。応答 `mtls`（`client_cert_pem` / `ca_cert_pem` / `expires_at`）。NotAfter ≈ Token soft TTL（x0068）。CA 未設定時は `csr_pem` を無視して Token のみ（Relay Agent 併存） | QUIC；zenohd 側の JWT；IP allowlist；Agent 退役必須化 |
| Room 鍵 AEAD 配布（x0069）: Room 作成時に 32B `room_key` + `wire_cap` を発行。Token **応答 JSON**（JWT claims 外）に `room_key`（URL-safe base64）/ `wire_cap` / `room_key_epoch` を同梱。Zenoh 平文の機密性正本は AEAD；`wire_cap` はルーティングのみ | `wire_cap` トピック名の秘匿；入室者同士の鍵 forward 秘匿；Allocation JWT への `room_key` 載せ |
| Room 鍵寿命（x007e）: `room_key_epoch` + Token JSON；visibility **縮小** / owner `rotate_room_key` / 定期（`ROOM_KEY_ROTATE_HOURS` 既定 24）で回転；直前世代は `RoomKeyGrace`（≈ Token TTL・メモリのみ）→ **host Token** に `room_key_prev*` として同梱（guest には載せない） | 3 世代以上の受理；world 再起動後の旧鍵；Allocation/Relay への `room_key` 配布；push 通知（host は ≈15s Token 再取得で採用） |
| client Home「新規ワールド作成」仮 UI でアクセスレベル選択→ create/PUT + 入室（x0037）；Worlds カタログ Join/Host での `room_id` retarget（`ROOM_ID` env・seal/attest・HostSupervisor 再起動 · x007c） | UI デザイン最終化；入室中の途中変更 UI（Leave なしの in-place 昇格）；テンプレ複数；カタログ全件の無制限走査 |
| `GET /.well-known/jwks.json`（Room Token 用） | verifier / ホスト死活監視 / S2S 連合 |
| Room API は auth-server Bearer JWT 必須（`sub` を Room Token に転写） | 匿名 Token（既定 OFF。`WORLD_ALLOW_ANON_TOKENS=true` のみ） |
| auth JWKS（`AUTH_JWKS_URL`）によるユーザー JWT 検証。共有キャッシュは失敗時もクールダウン + miss は非同期再取得（x008c；GenServer が HTTP で直列待ちしない） | auth の revocation 再確認；host 級 soft/hard TTL |
| HostAttestation JWT（`typ=host_attestation` + **`host_generation`**）を host Room Token のみ発行（x0025 / **x003a**）。同一 `host_pub` は generation 維持、鍵変更で bump。発行失敗は 5xx（token 検証失敗は 401） | UGC scene 編集；完全な handoff UX |
| ManifestSeal JWT（`typ=manifest_seal` / `aud=alchemy-manifest-seal` + **`host_generation`**）を host Room Token のみ発行（x0026 / **x0039** / **x003a**）。**DB scene + HostAttestation から canonical blake3 を再計算**し、host 提出 hash と不一致なら 422（署名 oracle 廃止）。`manifest-seal-hash --serve` 常駐 Port プール必須（x0075 / **x008d**；`PartitionSupervisor`；既定は **起動ホスト** `schedulers_online` / `MANIFEST_SEAL_HASH_PARTITIONS`）。`GET /ready` が Repo + Keys + 全 Port を検査（`/health` は liveness のみ・x008b） | UGC scene 編集；host ローカル改変 body への seal；パーティション数超の並列；open 失敗時の `/ready` ゼロ失敗；release ビルドホスト CPU 焼き付き |
| DemoScene（entries + loco + **初期 quat/scale**）を Room 作成時に紐付け。`GET …/scene` は **host** Room Token のみ（x0027 / x002c）。**x006d / x0082**: `POST …/scene/entries` で mesh 1 件追記（**`expected_revision` CAS**・revision bump・canonical seal 入力）。**x007b / x0082**: `DELETE …/scene/entries/:entity_id` で mesh 削除（64-hex blake3 必須・**`expected_revision` CAS**）。競合は **409** `scene_revision_conflict`（host が GET 後に有限再試行） | guest への scene 配布・フル UGC エディタ・collider scale；user_root / cuboid 削除；Room 単位グローバル直列化 alone；無限再試行 |
| PlaceRead JWT（`typ=place_read` / `aud=alchemy-assets-place-read`）を Room Token + 検証済み `world_seal` で発行（x006b）。scene blake3 は **ManifestSeal 時**固定。PlaceRead `exp` = **seal 絶対 unix `exp`**＋`touch_until`。期限切れ seal → `seal_expired`。再起動後 re-seal。`extra_blake3` 無効 | JWT 内全 hash 列挙；durable 永続／canonical 再 hydrate；PlaceRead が seal より長生き；退出即時 revoke；client 自動取得 |
| HTTP rate limit（x0071 / **x0084**）: auth-JWT scope は IP+user；host-token scope（`host_seal`=`host_keys`+`manifest_seal` / `host_scene` / `place_read`）は操作別 IP + **claims** `room_id\|jti` で 429。`place_read` 発行と hashes の jti は別 Bearer のため非連動（共有床は IP） | 分散ノード横断カウンタ；発行↔PlaceRead の jti 相関；JWT 署名検証前の jti 真正性 |

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

Token **応答 JSON**（JWT 外・同梱）: `wire_cap`（ルーティング）・`room_key` / `room_key_epoch`（x0069 / x007e）・任意 `mtls`（x0066）・host のみ任意 `room_key_prev*`（x007e）。Allocation JWT には `room_key` を載せない。

検証は host-server が JWKS を取得して行う（転送経路は Zenoh、権威は Token）。bind 時は Intent の `session_pub` と `cnf.spk` の一致が必須（盗聴 JWT だけでは別鍵 bind 不可）。HTTP credential（scene / attest / seal）は **host `role` + `cnf.spk` PoP**（x0081: `x-alchemy-pop-ts` / `x-alchemy-pop-sig`。盗んだ Token alone → 401）。

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
| GET | `/health` | 不要 | 生存確認（liveness・依存非検査） |
| GET | `/ready` | 不要 | 準備確認（Repo + Token.Keys + seal Port プール・x008b / x008d） |
| GET | `/.well-known/jwks.json` | 不要 | Room Token 検証用公開鍵 |
| GET | `/api/v1/rooms` | Bearer（auth JWT） | 見える Room カタログ（`?mine=1` で自分の部屋のみ・x007c；Public は active のみ・x0080） |
| GET | `/api/v1/rooms/:room_id` | Bearer（auth JWT） | 単室セッション要約（`state` / `active_host_sub`・秘密なし・x0080） |
| POST | `/api/v1/rooms` | Bearer（auth JWT） | Room 作成（任意 `visibility`；初期 `state=dormant`） |
| PUT | `/api/v1/rooms/:room_id/visibility` | Bearer（auth JWT） | owner のみ更新 |
| POST | `/api/v1/rooms/:room_id/tokens` | Bearer（auth JWT） | Token 発行（`session_pub` 必須・任意 `csr_pem`→`mtls`・x0066；応答に `wire_cap` / `room_key` / `room_key_epoch`・x0069/x007e；host は active_host / owner-resume・x0080） |
| POST | `/api/v1/rooms/:room_id/handover` | Bearer（auth JWT） | Graceful Handover（active_host または owner；`target_sub` null → dormant・x0080） |
| POST | `/api/v1/rooms/:room_id/takeover` | Bearer（auth JWT） | Manual Takeover CAS（dormant / lease 切れ・x0080） |
| POST | `/api/v1/rooms/:room_id/host_keys` | Bearer（**host Room Token**）+ PoP（x0081） | HostAttestation 発行（`host_pub` base64url・`host_generation`・x0025 / x003a） |
| POST | `/api/v1/rooms/:room_id/manifest_seal` | Bearer（**host Room Token**）+ PoP（x0081） | ManifestSeal 発行（`revision` + `host_generation` + `manifest_blake3` + `host_attestation`；canonical 照合・x0039 / x003a） |
| GET | `/api/v1/rooms/:room_id/scene` | Bearer（**host Room Token**）+ PoP（x0081） | DemoScene `{ revision, entries, locomotion }`（x0027） |
| POST | `/api/v1/rooms/:room_id/scene/entries` | Bearer（**host Room Token**）+ PoP（x0081） | mesh SceneEntry 追記 + **`expected_revision` CAS**（x006d / x0082） |
| DELETE | `/api/v1/rooms/:room_id/scene/entries/:entity_id` | Bearer（**host Room Token**）+ PoP（x0081） | mesh SceneEntry 削除 + **`expected_revision` body CAS**（x007b / x0082；blake3 必須） |
| POST | `/api/v1/rooms/:room_id/place_read` | Bearer（**Room Token**） | PlaceRead 発行（`world_seal` 必須・`extra_blake3` 無効・x006b） |
| GET | `/api/v1/rooms/:room_id/place_hashes` | Bearer（**PlaceRead**） | seal 時固定の scene blake3s（x006b） |

host-token / PlaceRead 系は x0084 で操作別 rate limit（超過は **429** + `retry-after`）。

## 永続（x0028）

| 項目 | 内容 |
|:---|:---|
| 既定 | AshSqlite。DB ファイルは `priv/data/world_dev.db`（test は `world_test.db`） |
| 上書き | env `WORLD_DATABASE_PATH`（**`MIX_ENV=test` では無視**・x003b；assets CAS 隔離と同型） |
| スキーマ | `rooms(room_id PK, owner_sub, scene JSON, visibility, active_host_sub, state, host_lease_expires_at, timestamps)` |
| Postgres 切替 | 属性は Postgres 互換のみ。運営規模拡大時は `ash_postgres` + `AshPostgres.DataLayer` / `AshPostgres.Repo` に差し替え、`mix ash.codegen` で再生成。API facade（`World.Rooms`）は変えない |
| 障害 | SQLite / Ash 失敗は `:storage_error` → HTTP 503 `storage_unavailable`（404/422 に潰さない） |
| prod volume | `world_prod_keys` に JWT PEM と SQLite を同居（`/var/lib/world`）。`-v` で両方消える |

## 起動（Docker）

auth-server を先に起動すること（JWKS 参照）。

```bash
cd auth-server && docker compose up -d && cd ..
cd world-server && docker compose up -d
# http://localhost:4003/health
# http://localhost:4003/ready
# AUTH_JWKS_URL 既定: http://host.docker.internal:4002/.well-known/jwks.json
# mix ash.setup が SQLite migrate を実行
```

**prod（x001e / x003b）:** `docker-compose.prod.yml` + `.env.prod`（`.env.prod.example` 参照）。  
`AUTH_INTERNAL_TOKEN`（auth/world 一致）と world の `AUTH_INTERNAL_URL` が必須。検証: `make smoke-prod-compose`（config）/ `SMOKE_PROD_UP=1 make smoke-prod-compose`（`/health`）。

## 関連

- ワイヤ: [protocol.md](protocol.md)（`InputEvent.room_token`）
- 転送: [relay-server.md](relay-server.md)
- 認証ユーザー JWT: [auth-server.md](auth-server.md)
