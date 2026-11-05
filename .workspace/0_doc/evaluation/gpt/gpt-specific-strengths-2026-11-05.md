# Alchemy 第2評価者 — 具体的な強み（2026-11-05）

## 採点基準

| 点数 | 基準 |
|:---:|:---|
| +1 | 正しく実装されている。問題はないが特筆するほどではない |
| +2 | 業界の一般的なベストプラクティスに沿った良い設計判断 |
| +3 | 同規模・同種プロジェクトの平均を明確に上回る実装 |
| +4 | プロダクションレベルのゲームエンジン・OSS と比較しても遜色ない実装 |
| +5 | このクラスの個人プロジェクトでは見たことがない卓越した実装 |

## 運営側コア

### auth-server

- **アイデンティティ SSoT と鍵ローテーション境界が明快** `+3`
  > User を auth に閉じ、RS256 `kid` は JWK thumbprint、active / verification keys を分離する。hot path は同期初期化済み `:persistent_term` を読み、GenServer 停止時は stale term が残っても fail-closed にする（`keys.ex:1-14, 45-89, 92-137`）。
  > 対象ファイル: `auth-server/lib/auth/token/keys.ex`

- **公開 API・S2S・rate limit の責務分離が実用水準** `+2`
  > login/register/refresh/social mutate は IP と主体の bucket を持ち、`/internal/*` は public Tunnel で 404、prod internal token 未設定は起動失敗という二重境界を採る。今回の Docker CI でも throttle 否定系を含む 130 tests が通過した。
  > 対象ファイル: `auth-server/lib/auth_web/router.ex`, `auth-server/lib/auth/rate_limit.ex`, `edge/cloudflared/config.yml.example`

### assets-server

- **物理 CAS・所有参照・読取権利を分離した認可モデル** `+4`
  > `CasObject` は bytes meta のみ、Quota は owner×hash の `BlobRef`、GET は `BlobGrant` または seal 時 scene に束縛された PlaceRead に限定する（`blobs.ex:1-16, 42-49, 420-469`）。前回の access 先着固定はコード上で解消され、同一 bytes の cross-owner 共存と認可を両立した。
  > 対象ファイル: `assets-server/lib/assets/blobs.ex`

- **Quota 予約と CAS 書込を跨ぐ補償が共有 bytes の競合を壊さない** `+3`
  > 短い DB 予約 TX → TX 外 CAS put → finalize の順で、失敗時は予約だけを戻して CAS bytes を削除しない（`blobs.ex:117-188, 190-270`）。same-hash 並行 PUT で片方の補償が他方の bytes を消す典型的 TOCTOU を避けている。
  > 対象ファイル: `assets-server/lib/assets/blobs.ex`

- **予約 reaper / orphan GC が quarantine と再検証を備える** `+3`
  > GC は候補の grace、CasObject / reservation 再確認、quarantine、最終 callback、失敗時 restore を組み合わせる（`orphan_gc.ex:1-13, 95-188`）。単純な list-and-delete より安全で、クラッシュ中断も再試行可能である。
  > 対象ファイル: `assets-server/lib/assets/cas/orphan_gc.ex`

### world-server

- **中央定義から canonical ManifestSeal を発行する信頼モデル** `+4`
  > host 提出 hash を署名せず、world DB scene・attested host key・generation から canonical hash を再計算する。Room Token、HostAttestation、ManifestSeal、PlaceRead の audience / typ を分け、host を実行係に留めている。
  > 対象ファイル: `world-server/lib/world/canonical_manifest.ex`, `world-server/lib/world_web/controllers/room_controller.ex`

- **CSR の主体名を信用せず role CN をサーバー側で強制する** `+3`
  > world mTLS issuer は client private key を受け取らず、CSR Subject を `alchemy-host` / `alchemy-guest` へ置換し、NotAfter を 300 秒へ合わせる（`mtls_issuer.ex:1-8, 28-60, 215-248`）。前回の 24 時間証明書問題を正しい境界で閉じた。
  > 対象ファイル: `world-server/lib/world/mtls_issuer.ex`

- **Room API が永続障害と認可不一致を混同しない** `+2`
  > `:not_found` / `:forbidden_host` / `:storage_error` を facade で分け、token 発行が暗黙に Room を作らない。Docker CI 116 tests も通過した。
  > 対象ファイル: `world-server/lib/world/rooms.ex`, `world-server/lib/world_web/controllers/room_controller.ex`

## 共通契約・ホスト実行

### protocol

- **ワイヤ契約に byte/count 上限・世代ゲート・AEAD を集約** `+4`
  > protobuf 型だけでなく、class 別 decode 上限、`(host_generation, revision)`、Room ID / wire_cap 検証、XChaCha20-Poly1305 AAD を共有 crate に置く。wrong room / kind / key、oversize、非有限値を単体テストし、今回 67 tests が通過した。
  > 対象ファイル: `protocol/src/decode_limits.rs`, `protocol/src/generation_gate.rs`, `protocol/src/wire_aead.rs`

- **定義と実行の境界がプロトコル上で保たれる** `+3`
  > SceneManifest が entity 定義、RenderFrame が pose delta、InputEvent が Intent、Presence が割当を担当し、GLB 本体をワイヤへ載せない。`PROTOCOL_PIN` と配送クラスも明示される。
  > 対象ファイル: `protocol/proto/scene_manifest.proto`, `protocol/proto/render_frame.proto`, `protocol/peer-delivery.md`

### client/host-server

- **Room Token bind 後を Ed25519 session に落とす入力門番** `+4`
  > `cnf.spk` と session key を束縛し、steady Intent は単調 seq + signature、finite / magnitude / Grab 距離 / per-source rate を通す。JWT を毎 tick 解析せず、盗聴 Token 単独の再利用も防ぐ。
  > 対象ファイル: `client/host-server/src/room_token/session.rs`, `client/host-server/src/intent_gate.rs`

- **60 Hz 物理と 30 Hz 配信、HTTP と publish を worker へ分離** `+3`
  > 物理 loop は credential HTTP と Zenoh encode/put を直接行わず、pose job と channel で worker に委譲する。tick の保証単位が network / credential failure から分離されている。
  > 対象ファイル: `client/host-server/src/main.rs`, `client/host-server/src/network_worker.rs`, `client/host-server/src/credential_worker.rs`

- **共有 scene の revision 単調適用が stale 応答を物理へ戻さない** `+2`
  > spawn/despawn HTTP が順不同に完了しても、physics thread は incoming revision が strictly greater の場合だけ scene と Rapier を更新する（`room_scene.rs:129-145`, `inventory_spawn.rs:250-274`）。
  > 対象ファイル: `client/host-server/src/room_token/room_scene.rs`, `client/host-server/src/inventory_spawn.rs`

## クライアント・プロセス管理

### client/app

- **GUI と headless E2E が同じ trust gate を使う** `+4`
  > `alchemy-client-trust` が AEAD→identity→seal→attestation→gen/rev を Accept / Discard として実装し、GUI subscriber も trust-probe も同じ関数を呼ぶ（`trust/src/lib.rs:1-22`, `app/src/bus/manifest.rs:79-121`）。前回の「probe が実 client path を通らない」は解消した。
  > 対象ファイル: `client/trust/src/lib.rs`, `client/app/src/bus/manifest.rs`, `client/trust/src/bin/trust-probe.rs`

- **検証失敗を Leave に変換しない状態機械** `+3`
  > forged seal / wrong key / stale generation は watermarks を変えず discard し、参加状態を維持する。WAN probe は wildcard で他室 ciphertext が届く現実もテストし、own key で復号不能を確認する（`trust-probe.rs:350-465, 550-647`）。
  > 対象ファイル: `client/trust/src/manifest.rs`, `client/trust/src/bin/trust-probe.rs`

- **Desktop / OpenXR の切替を surface に閉じ、wgpu 経路を共有** `+3`
  > READY 前の Desktop 入力所有、presence hysteresis、manual F8、XR panic fallback を surface 層に閉じる。Room や auth の概念を UI Facet へ漏らさず、旧 Vulkan 二重 loop も除去済みである。
  > 対象ファイル: `client/app/src/surface/hotswitch.rs`, `client/app/src/surface/xr/wgpu_interop.rs`

### launcher

- **固定 discovery endpoint で host 再起動後の IPC stale を解消** `+3`
  > host crash 時は discovery を即 invalidate し、ready 後に generation と新 IPC を publishする（`main.rs:140-170`）。client は終了直前に LOOKUP でき、意図停止と crash restart も分ける。launcher 8 tests が通過した。
  > 対象ファイル: `launcher/src/main.rs`, `launcher/src/discovery.rs`

## 転送・横断品質

### relay / edge

- **短命 mTLS・default deny・非対称 Zenoh ACL の実装** `+3`
  > zenohd は JWT を持たず、mTLS role CN と key-family ACL に限定する。guest の sim put ingress を拒否しつつ、Zenoh の interest egress を明示する設定は transport の実挙動を踏まえている（`config.wan.json5:45-216`）。
  > 対象ファイル: `relay-server/config.wan.json5`

- **Relay Agent の寿命・body・thread・JWKS・jti 境界をテストで固定** `+3`
  > NotAfter、16 KiB body、接続 / in-flight 上限、hard-stale JWKS、unknown-kid cooldown、jti dedupe が実装され（`lib.rs:30-44, 347-477, 597-686`）、今回 16 tests が通過した。前回 P0 の大部分を直接閉じた。
  > 対象ファイル: `relay-agent/src/lib.rs`

- **Cloudflare Tunnel の公開面が明示的に絞られる** `+2`
  > `/dev/*` と auth `/internal/*` を edge で 404 にし、origin は loopback の prod service に限定する（`config.yml.example:7-30`）。
  > 対象ファイル: `edge/cloudflared/config.yml.example`

### テスト・可観測性・自己改善

- **独立 crate を含む CI と否定系 E2E が揃った** `+4`
  > Rust workspace、protocol、launcher、relay-agent、cargo-deny、fuzz、Elixir 3 service、hermetic/WAN E2E が workflow に分かれ、path filter に実コードが列挙される。relay-agent の CI 漏れは解消した。
  > 対象ファイル: `.github/workflows/rust-ci.yml`, `.github/workflows/e2e-compose.yml`, `.github/workflows/host-fuzz-ci.yml`

- **低 cardinality metrics と jti 相関が主要門番へ広がった** `+3`
  > auth/world/relay と host/client/assets に loopback metrics を持ち、Room Token `jti` と Allocation `room_jti` をログ相関する。秘密や `sub` を metric label にしない判断も妥当である。
  > 対象ファイル: `world-server/lib/world/metrics.ex`, `relay-agent/src/metrics.rs`, `client/host-server/src/metrics.rs`

- **保証 / 非保証と回帰スライスをコード近傍に維持する規律** `+3`
  > architecture は現状と目標を分け、x0074〜x0079 の done を reaper、常駐 Port、分割、rate source、temp ACL、共有 vector のコードとテストで追跡できる。継続加点ではなく、今回再読できた境界に限定して評価した。
  > 対象ファイル: `.workspace/0_doc/architecture/`, `.workspace/roadmap/slices/`

**強み合計: +71**
