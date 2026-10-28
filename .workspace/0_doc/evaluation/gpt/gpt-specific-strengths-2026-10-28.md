# Alchemy 第2評価者 — 具体的な強み（2026-10-28）

## 採点基準

| 点数 | 基準 |
|:---:|:---|
| +1 | 正しく実装されている |
| +2 | 一般的なベストプラクティスに沿う良い設計 |
| +3 | 同規模・同種プロジェクトの平均を明確に上回る |
| +4 | プロダクション級 OSS と比較しても遜色ない |
| +5 | 個人プロジェクトとして卓越している |

## 運営側コア

### auth-server

- **アカウントライフサイクルと失効を一つの権威に集約** `+4`
  > Argon2、登録・ログイン、access/refresh、logout、password reset/change、deactivate を auth-server に閉じ、他サービスは `sub` のみを参照する。鍵は `kid` 付き RS256/JWKS で複数 verification key を公開できる。
  > 対象ファイル: `auth-server/lib/auth/accounts.ex`

- **Refresh Token の競合を DB ロックと条件付き失効で閉じた** `+3`
  > transaction 内の `FOR UPDATE` 取得と `revoked_at IS NULL` 条件付き revoke により、前回指摘した同一 token の二重 rotation を実装上防いでいる。競合 API 契約テストも追加された。
  > 対象ファイル: `auth-server/lib/auth/accounts.ex`

- **Social SSoT と visibility 判定を分離した** `+3`
  > Follow、相互 Follow 後の Friendship、unfollow と friendship 解除の同一 transaction、Friends+/Followers+ 判定を auth に置き、world からは共有 secret 付き S2S predicate として参照する。関係グラフを JWT に肥大化させない判断も妥当。
  > 対象ファイル: `auth-server/lib/auth/social.ex`

### assets-server

- **DB メタと local/S3 CAS の境界が明確** `+2`
  > BLAKE3 キーの不変バイナリを CAS に、owner/size/mime/quota を SQLite に分離し、S3 時は短命 presigned URL の 302 で BEAM を大容量転送経路から外す。
  > 対象ファイル: `assets-server/lib/assets/cas.ex`

- **Quota 消費を条件付き UPDATE にした** `+2`
  > `used_bytes + add_bytes <= limit_bytes` を where 条件に含めた単一 UPDATE により、前回の read-check-increment race を解消した。test 環境も runtime の S3 env から隔離された。
  > 対象ファイル: `assets-server/lib/assets/blobs.ex`

### world-server

- **Room owner・永続・host capability を門番で強制** `+3`
  > Token endpoint は room を暗黙作成せず、Ash/SQLite の owner を読み、`role=host` は owner のみに限定する。storage failure を 404 に潰さず 503 に写像する点も良い。
  > 対象ファイル: `world-server/lib/world_web/controllers/room_controller.ex`

- **7段階 visibility を fail-closed で実装** `+2`
  > private/public/LAN/social を分離し、social 判定の auth S2S 障害は許可ではなく 503、LAN は Cloudflare ingress を拒否する。既発行 Token の TTL-only という非保証も明記されている。
  > 対象ファイル: `world-server/lib/world/room_access.ex`

- **Room Token・HostAttestation・ManifestSeal の用途を分離** `+2`
  > audience と `typ` を分け、host HTTP capability、host 公開鍵、Manifest body hash を別 JWT にした。guest token から attest/seal/scene を取得できないテストもある。
  > 対象ファイル: `world-server/lib/world/host_attestation.ex`

## 共通契約・ホスト

### protocol

- **独立 crate と PROTOCOL_PIN を契約正本にした** `+2`
  > SceneManifest、RenderFrame、InputEvent、PresenceState を頻度と責務で分け、client workspace 外の単一 crate から共有する。全4 schema は CI で protoc compile される。
  > 対象ファイル: `protocol/src/lib.rs`

- **Ed25519 session と host 署名を canonical protobuf に結合** `+3`
  > Intent は session key・単調 seq、Frame/Presence は attested host key で署名する。署名 field を空にして encode する規則が一箇所にあり、改変テストもある。
  > 対象ファイル: `protocol/src/wire_crypto.rs`

- **room_id・配送・capability の層間契約を共有** `+2`
  > 64-byte allowlist validator、key helper、tick gap decision、role→capability、user-root 定数を共有し、前回の validator/ID 重複を縮小した。
  > 対象ファイル: `protocol/src/lib.rs`

### client/host-server

- **Untrusted input gate が多段化された** `+3`
  > Room Token、Ed25519 session bind、単調 seq、role capability、finite、単位球 clamp、Grab 距離を順に強制する。巨大 finite move と遠距離 Grab の回帰テストがある。
  > 対象ファイル: `client/host-server/src/session.rs`

- **Rapier 60Hz と純デルタ 30Hz の実行モデル** `+3`
  > SoA 寄りの simulation、固定 physics tick、静止物を省く pose delta、最後に publish した tick を使う snapshot を備える。Manifest 分類は shared-sim に寄せられた。
  > 対象ファイル: `client/host-server/src/main.rs`

- **Presence と長時間 credential のライフサイクル** `+3`
  > Leave、`sub+jti` liveliness、TTL の三経路を同じ reclaim に収束し、auth→host token→attestation→seal を期限前更新する。mutex poison も空状態へ偽装せず fatal にした。
  > 対象ファイル: `client/host-server/src/main.rs`

## クライアント・配布境界

### client/app

- **Frame の真正性・有限値・順序・再同期を実装** `+3`
  > attested host key の署名、finite、stale/gap tick を検証し、欠落 delta は適用せず署名付き snapshot で全置換する。前回の「偽 Frame を直接描画」「収束手段なし」は解消した。
  > 対象ファイル: `client/app/src/main.rs`

- **Manifest の world seal と HostAttestation を検証** `+2`
  > protocol pin、room、seal body hash、host key attestation を通るまで host key を採用せず、host 世代変更時は world/session をリセットする。
  > 対象ファイル: `client/app/src/main.rs`

- **Desktop/OpenXR と非同期 UI job の分離** `+2`
  > surface 層が runtime fallback と入力差を吸収し、login/register/create/join は描画 event thread から background job へ移された。
  > 対象ファイル: `client/app/src/surface/desktop.rs`

- **Manifest 駆動描画と GPU slot 分離** `+2`
  > shared-sim の EntityKind、Frame TRS、box/remote の予約済み別 UBO slot を用い、前回の uniform alias を解消した。
  > 対象ファイル: `client/app/src/scene.rs`

### launcher

- **最小プロセスマネージャを実装** `+2`
  > host ready 後の client 起動、client-only exit、loopback one-time-token IPC、非0 crash の bounded restart、Ctrl+C shutdown を実装した。host 自身が port 0 bind するため TOCTOU も避ける。
  > 対象ファイル: `launcher/src/main.rs`

### zenoh-router / edge

- **WAN を平文 TCP から TLS に移行** `+1`
  > WAN config は multicast を切り、`tls/0.0.0.0:7447` と server certificate を要求する。前回の平文盗聴リスクは解消した。
  > 対象ファイル: `zenoh-router/config.wan.json5`

- **HTTPS 公開面を Tunnel template で限定** `+2`
  > credential を repository 外に置き、auth/world/assets のみを origin へ転送し、`/dev/*` と catch-all を 404 にする。
  > 対象ファイル: `edge/cloudflared/config.yml.example`

## 横断品質

### テスト戦略

- **Hermetic Compose 縦 E2E を必須 CI に追加** `+3`
  > auth login→Room Token→signed Intent→host→signed Frame/snapshot を Compose で通し、関連サービス変更時に起動する。
  > 対象ファイル: `.github/workflows/e2e-compose.yml`

- **境界の回帰テストが大幅に増えた** `+2`
  > 2026-10-28 の実行で protocol 20 tests、Rust workspace の client/shared/host 等が全成功し、host-server 単体だけで57 testsが通過した。
  > 対象ファイル: `client/host-server/src/main.rs`

### 可観測性・エラー・保守性

- **host の集約カウンタと理由付きログ** `+1`
  > physics steps、publish/upsert/empty delta、input drop、credential/JWKS failure に文脈があり、試作段階の手動診断には有効。
  > 対象ファイル: `client/host-server/src/main.rs`

- **外部障害を fail-closed と 5xx に分離** `+2`
  > world storage/auth S2S、credential refresh、seal refresh、JWT/JWKS の失敗を認可成功や not-found に潰さない。
  > 対象ファイル: `world-server/lib/world_web/controllers/room_controller.ex`

- **shared-sim と共通 auth client で重複知識を削減** `+3`
  > scene 分類・interaction 定数を shared-sim、JWKS 検証を共通 crate に寄せ、unsafe block は workspace lint で理由コメントを必須にした。
  > 対象ファイル: `client/shared-sim/src/lib.rs`

### DX・セキュリティ・全体設計

- **Docker/WSL/個別 CI/smoke の導線** `+2`
  > README から日常 stack、launcher、個別 CI、WAN TLS/Tunnel まで到達でき、Elixir native toolchain 差を Docker で吸収する。
  > 対象ファイル: `README.md`

- **依存脆弱性 gate を CI に導入** `+2`
  > Elixir 3サービスは `mix hex.audit`、Rust は cargo-deny を必須 job とし、前回の「advisory があっても通過」を改善した。
  > 対象ファイル: `.github/workflows/rust-ci.yml`

- **定義・門番・実行・endpoint の責務が実コードに降りた** `+3`
  > protocol=定義、auth/assets/world=永続権威と門番、host=実行、client=endpoint、launcher=process ownership という大枠は、trust-boundary 文書とコードで整合している。
  > 対象ファイル: `.workspace/0_doc/architecture/trust-boundary.md`

**強み合計: +69**
