# Alchemy — プラス点詳細（第1評価者 Claude Opus 5.5 / 2026-11-12）

> 評価対象コミット: `8833821`（main・PR #187 / x009c 直後）。前回まとめ時点 `c491016` から 122 コミット。
> 「継続」項目は、前回根拠ファイルについて `git diff c491016 HEAD` で変更有無を確かめ、変更があったファイルは当日再読した（例: auth-server は 5 ファイル・+144 行のみで、継続項目の根拠コードは不変）。「新規」項目はすべて当日コードを読んで確認した。
> 第2評価者（`gpt/`）の文書は参照していない。

## 採点基準

| 点数 | 基準 |
|:---:|:---|
| +1 | 正しく実装されている。問題はないが特筆するほどではない |
| +2 | 業界の一般的なベストプラクティスに沿った、良い設計判断 |
| +3 | 同規模・同種プロジェクトの平均を明確に上回る実装 |
| +4 | プロダクションレベルのゲームエンジン・OSSと比較しても遜色ない実装 |
| +5 | このクラスの個人プロジェクトでは見たことがないレベルの卓越した実装 |

| ブロック | 観点 | 小計 |
|:---|:---|---:|
| 運営側コア | auth-server | +26 |
| 運営側コア | assets-server | +35 |
| 運営側コア | world-server | +52 |
| 共通契約 | protocol/ | +40 |
| 共通契約 | client/shared-sim | +4 |
| ホスト端末 | client/host-server | +48 |
| ホスト端末 | client/app（HostSupervisor 含む） | +52 |
| ホスト端末 | launcher/ | 対象外（退役） |
| 転送・エッジ | relay-server / edge / relay-agent | +24 |
| 横断 | テスト戦略 | +30 |
| 横断 | 可観測性 | +10 |
| 横断 | エラーハンドリング | +7 |
| 横断 | 変更容易性・保守性 | +13 |
| 横断 | 開発者体験（DX） | +10 |
| 横断 | セキュリティ・配布可能性 | +17 |
| 横断 | プロジェクト全体設計 | +20 |
| | **合計** | **+388** |

---

## 技術評価層 — 運営側コア

### auth-server（アイデンティティ SSoT）

- **Refresh Token のファミリー回転・再利用検知・行ロック付き回転（継続）** `+4`
  > Ory Kratos / Keycloak の最小構成と比べても遜色ない。根拠コードは前回から不変。
  > 対象ファイル: `auth-server/lib/auth/`

- **RS256 + `kid` + 検証鍵ローテ、署名鍵を `:persistent_term` に置き未準備時 fail-closed（継続）** `+4`
  > 前回 x0073 で直列化を解消した形のまま。
  > 対象ファイル: `auth-server/lib/auth/token/keys.ex`

- **多軸 rate limit（IP × 識別子 × トークンファミリー）・Social mutate の両軸 RL・Tunnel 背後 IP 解決の共有（継続）** `+6`
  > `shared/alchemy_client_ip` / `alchemy_rate_limit` を auth / world / assets で共有する構図は不変。当日 shared 3 パッケージ 25 テスト PASS。
  > 対象ファイル: `auth-server/lib/auth_web/`, `shared/alchemy_client_ip/`, `shared/alchemy_rate_limit/`

- **アカウント列挙・タイミング攻撃への配慮、メール確認 / パスワードリセットのトランザクション設計（継続）** `+4`
  > 対象ファイル: `auth-server/lib/auth/`

- **失敗分類ログ・`access_jti` metadata・S2S 内部トークンの prod fail-closed・パイプライン分離・loopback metrics（継続）** `+5`
  > 対象ファイル: `auth-server/lib/auth_web/router.ex`, `auth-server/lib/auth_web/plugs/`

- **Follow / Friendship の DB 制約付きソーシャルグラフ（継続）** `+2`
  > 対象ファイル: `auth-server/lib/auth/`

- **`/ready`（Repo・Keys）を liveness と分離（新規・x008b）** `+1`
  > `Auth.Ready` が Repo と署名鍵の準備を見て 503 を返す。テスト 53 行追加。
  > 対象ファイル: `auth-server/lib/auth/ready.ex`, `auth-server/lib/auth_web/controllers/health_controller.ex`

### assets-server（CAS・Quota・門番）

- **CAS 物理から access を剥がし、GET を常時 Grant / PlaceRead で門番（継続）** `+7`
  > `authorize_and_fetch/4` は `auth_sub` の Grant または covering PlaceRead のみで許可（`blobs.ex` L430-470）。
  > 対象ファイル: `assets-server/lib/assets/blobs.ex`

- **reserved reaper + orphan CAS GC の quarantine → 再確認 → discard / restore（継続）** `+3`
  > 対象ファイル: `assets-server/lib/assets/quota/reserved_reaper.ex`, `assets-server/lib/assets/cas/orphan_gc.ex`

- **Quota 予約 → TX 外 CAS put → 確定、条件付き UPDATE による原子更新（継続）** `+5`
  > 対象ファイル: `assets-server/lib/assets/blobs.ex`

- **検疫を `Assets.Quarantine.Worker` Behaviour + `public_reason/1` に分離（継続・ライセンス境界準拠）** `+2`
  > pgvector 等の Proprietary は OSS ツリーに無い。
  > 対象ファイル: `assets-server/lib/assets/quarantine/`

- **BLAKE3 を常駐 Port で計算し、`PartitionSupervisor` で N 本プール化（継続 + 新規・x008d）** `+4`
  > 前回 -1 だった単一 GenServer 直列を、新規依存なしの `PartitionSupervisor` + `unique_integer` ラウンドロビンで解消（`blake3/worker.ex` L4-10, L35-44）。`/ready` は全パーティションの Port 準備を見る（L64-72）。test は `blake3_hash_partitions: 2` で多 Port 経路を実際に通す（`config/test.exs` L36-37）。
  > 対象ファイル: `assets-server/lib/assets/blake3/worker.ex`, `assets-server/lib/assets/application.ex`（L23-28）

- **メタと不変バイナリの分離・CAS backend behaviour・s3 時 302（継続）** `+6`
  > 対象ファイル: `assets-server/lib/assets/cas/`

- **Drive / VCS 薄皮で take_out・put_back の全 blake3 に Grant を要求（継続）** `+1`
  > `assert_blob_grant/2` と `assert_child_grants/2`（`drive.ex` L278-335）。Prefab 多子でも子ごとに Grant を見る。
  > 対象ファイル: `assets-server/lib/assets/drive.ex`

- **Origin Cert を内容束縛 v1 に（新規・x0096）** `+2`
  > 署名対象を `v1|{target_kind}|{target_id}|{subject_sub}|{tree_blake3}|{parent_commit_id}` に固定し（`vcs/cert.ex` L14-23, L222-236）、verify は tree / parent / 署名の不一致や対象消失を `:tampered` で fail-closed（L135-175）。連合前に署名ペイロードの形とバージョン接頭辞を決めた判断が良い。
  > 対象ファイル: `assets-server/lib/assets/vcs/cert.ex`

- **GET を `application/octet-stream` + `nosniff` 固定（302 にも付与）（新規・x008b）** `+2`
  > `CasObject.mime` を Content-Type に一切反映しない（`blob_controller.ex` L36-50）。first-writer mime squatting の足場が消えた。
  > 対象ファイル: `assets-server/lib/assets_web/controllers/blob_controller.ex`

- **生 body 受信・local CAS の tmp → rename・テスト環境の CAS env 無視（継続）** `+3`
  > 対象ファイル: `assets-server/lib/assets/cas/local.ex`, `assets-server/config/test.exs`

### world-server（案内所・門番・監査）

- **ManifestSeal を DB の scene + 検証済み attestation から world 自身が計算（継続）** `+4`
  > `create_manifest_seal` は host 申告の revision / blake3 を照合のみに使い、canonical 値を `CanonicalManifest.canonical_blake3/2` で再計算（`room_controller.ex` L1013-1027）。
  > 対象ファイル: `world-server/lib/world_web/controllers/room_controller.ex`

- **CSR から短命 mTLS を署名し CN をサーバーが強制（継続）** `+3`
  > 対象ファイル: `world-server/lib/world/mtls_issuer.ex`

- **PlaceRead の `exp` を検証済み seal の絶対 `exp` に一致（継続）** `+3`
  > 対象ファイル: `world-server/lib/world/place_hashes.ex`

- **HostAttestation / ManifestSeal の typ・aud 分離 RS256 と host_generation（継続）** `+3`
  > 対象ファイル: `world-server/lib/world/host_attestation.ex`, `world-server/lib/world/manifest_seal.ex`

- **host 専用 HTTP に Ed25519 PoP（`cnf.spk`）を必須化（新規・x0081）** `+4`
  > `World.HttpPop.verify/5` が `alchemy/pop/v1|{METHOD}|{path}|{ts}|{raw_body}` を JWT の `cnf.spk` で検証し、±60s skew を拒否（`http_pop.ex` L5-8, L35-47, L54-57）。生 body は JSON デコード前に `CacheBodyReader` で退避し、partial（`{:more, …}`）は退避しない＝検証失敗に倒れる（`cache_body_reader.ex` L7-15）。host_keys / scene GET / scene append・delete / manifest_seal の全 5 経路が `require_host_token` の直後に `require_http_pop` を通る（`room_controller.ex` L635, L730, L806, L901, L1017）。署名バイト列は `alchemy_protocol::http_pop` と lockstep。前回 -3 の「盗んだ host Token で乗っ取れる連鎖」を、Token を盗まれても鍵が無ければ 401 という形で閉じた。RFC 9449 DPoP の JWT 形を採らず、既存の session 鍵を再利用した判断も筋が良い。
  > 対象ファイル: `world-server/lib/world/http_pop.ex`, `world-server/lib/world_web/cache_body_reader.ex`, `world-server/lib/world_web/controllers/room_controller.ex`（L1441-1469）

- **Room 鍵 epoch（CAS ローテ・host 限定 grace・定期ローテ・deploy baseline）（新規・x007e）** `+4`
  > visibility が**縮小**したときは visibility 更新と鍵ローテを同じ CAS UPDATE で行い（`rooms.ex` L496-526, L1417-1430）、旧鍵は `RoomKeyGrace`（ETS・TTL = Room Token TTL）に置いて **host Token にだけ** `room_key_prev` を載せる（`room_key_grace.ex` L1-15、`room_controller.ex` L460-478）。`RoomKeyRotator` は `ROOM_KEY_ROTATE_HOURS`（既定 24）で古い部屋をローテし、`room_key_rotated_at` が NULL の行は「回さずに刻印だけ」してデプロイ直後の一斉ローテ（stampede）を避ける（`rooms.ex` L1541-1580）。CAS 負けは鮮度を見て再試行を打ち切る（L1466-1538）。MLS 的な鍵寿命の考え方を、共有鍵モデルのまま最小コストで入れた。
  > 対象ファイル: `world-server/lib/world/rooms.ex`, `world-server/lib/world/rooms/room_key_grace.ex`, `world-server/lib/world/rooms/room_key_rotator.ex`

- **canonical scene の revision CAS（新規・x0082）** `+2`
  > `UPDATE rooms … WHERE room_id = ? AND CAST(json_extract(scene, '$.revision') AS INTEGER) = ?`（`rooms.ex` L270-300）。0 行時に「部屋が消えた（404）」と「CAS 負け（409）」を再取得で区別し、host は 409 で再試行する。別カラムを足さずに JSON 内 revision で CAS を取ったのは SQLite 単一構成（Stage 1）に合っている。
  > 対象ファイル: `world-server/lib/world/rooms.ex`

- **host-token 系 rate limit の jti 軸を Bearer claims から取る（新規・x0084）** `+2`
  > `host_seal` / `host_scene` / `place_read` を操作別バケットに分け、jti 軸は path の room_id ではなく Bearer claims の `room_id|jti` から作る（`plugs/rate_limit.ex` L6-7, L121-132）。path を回してカウンタを分割する回避を潰している。
  > 対象ファイル: `world-server/lib/world_web/plugs/rate_limit.ex`, `world-server/config/config.exs`（L51-75）

- **永続オーナーと現計算ホストを分離し、lease を CAS で管理（新規・x0080）** `+3`
  > `activate_as_host` は「自分の renew」か「dormant / lease 切れからの claim」だけを許し、生きている host を奪えない（`rooms.ex` L1022-1046）。`lease_view/1` は書き込みなしで期限切れを dormant に見せ、GET ポーリングを安全にする（L1013-1020）。handover / takeover / sweep もすべて 1 文 UPDATE の CAS。※ scene 書き込み権限の付け方は weaknesses の -3 を参照。
  > 対象ファイル: `world-server/lib/world/rooms.ex`（L801-1111）

- **Room 上限を quota 行 + room insert の単一 TX（BEGIN IMMEDIATE）に（新規・x0097）** `+1`
  > 並行作成の overshoot を閉じた（`rooms.ex` L1267-1290、`rooms/owner_quota.ex`）。
  > 対象ファイル: `world-server/lib/world/rooms/owner_quota.ex`

- **Worlds catalog が秘密を返さず、owned を global top-N で落とさない（新規・x007c）** `+2`
  > `list_visible_summaries/3` は room_key / Token / wire_cap / scene を含まず（`rooms.ex` L552-560）、owned を先に確保してから public 候補を ACL 評価する。
  > 対象ファイル: `world-server/lib/world/rooms.ex`

- **seal-hash Port のプール化と `/ready`（新規・x008d / x008b）** `+2`
  > `/health` は liveness のまま、`/ready` が Repo・Token.Keys・全 seal Port を見て 503 を返す（`health_controller.ex` L10-27、`manifest_seal_hash/worker.ex` L60-66）。
  > 対象ファイル: `world-server/lib/world_web/controllers/health_controller.ex`, `world-server/lib/world/manifest_seal_hash/worker.ex`

- **Room 鍵の CSPRNG 発行と Allocation JWT 非搭載（継続）** `+2`
  > `room_controller.ex` L253-254 のコメントどおり Allocation JWT は鍵も epoch も持たない。
  > 対象ファイル: `world-server/lib/world_web/controllers/room_controller.ex`

- **`:user_auth` 後段の IP / user 両軸 rate limit（継続・handover / takeover にも適用）** `+2`
  > 対象ファイル: `world-server/lib/world_web/plugs/rate_limit.ex`（L59-72）

- **Room Token に `cnf.spk` を埋め込み、発行時に検証（継続）** `+2`
  > 対象ファイル: `world-server/lib/world/token.ex`（L5-29, L77-84）

- **Ash + SQLite 永続化とストレージ障害の 503 分離、公開範囲 ACL の S2S fail-closed、Room Token claim + jti metadata、auth JWT `sub` 転写（継続）** `+8`
  > 対象ファイル: `world-server/lib/world/`, `world-server/lib/world/auth_access.ex`

- **Verifier を `AuditEngine` Behaviour の注入点に留め、Zenohex を adapter 1 モジュールに隔離（継続）** `+2`
  > ライセンス境界ルールどおり Spot-Check 統計は OSS ツリーに無い。
  > 対象ファイル: `world-server/lib/world/verifier/`, `world-server/lib/world/network/zenoh/zenohex_adapter.ex`

- **`wire_cap` と Allocation Token の分離・`lan` fail-closed・匿名 Token 既定 OFF（継続）** `+3`
  > 対象ファイル: `world-server/lib/world/allocation_token.ex`, `world-server/lib/world/room_access.ex`

---

## 技術評価層 — 共通契約層

### protocol/（ワイヤ契約 SSoT）

- **Room 鍵 AEAD を v2 に上げ、AAD に epoch を入れ、current → prev の開封を純関数で提供（継続 + 新規・x007e）** `+5`
  > AAD = `alchemy/aead/v2|{room_id}|{epoch}|{kind}`（`wire_aead.rs` L31, L94-96）で他室・他 epoch・他 kind への流用を拒否。grace は `open_current_or_prev`（L160-175）として protocol に置き、「どれだけ猶予するか」は host の方針に残した（L12）。`wrong_epoch_fails` テストあり（L266）。
  > 対象ファイル: `protocol/src/wire_aead.rs`

- **制御面署名の canonical bytes（f64 を `to_bits` hex で固定）（新規・x0081）** `+2`
  > `alchemy/control/v1|spawn|{room_id}|{seq}|{blake3}|{pose×10 hex}`（`control_plane.rs` L14-27）。f64 の文字列化揺れを避け、room_id と単調 seq を署名に含めてリプレイと他室流用を防ぐ。
  > 対象ファイル: `protocol/src/control_plane.rs`

- **HTTP PoP の署名バイト列を protocol と Elixir で lockstep（新規・x0081）** `+2`
  > `protocol/src/http_pop.rs` と `World.HttpPop.signing_bytes/4` が同じ形式。
  > 対象ファイル: `protocol/src/http_pop.rs`

- **decode 上限・`(generation, revision)` 単調ゲート・チャネル責務分離・`wire_crypto` 集約・配送クラス契約（継続）** `+15`
  > 対象ファイル: `protocol/src/decode_limits.rs`, `protocol/src/lib.rs`, `protocol/peer-delivery.md`

- **Port framing・`PROTOCOL_PIN`（`0.21.0-x009b`）・TRS 正規化・`sim_capability`・独立クレート + protoc 検査・キー式ヘルパ（継続）** `+12`
  > 対象ファイル: `protocol/src/lib.rs`（L58）, `protocol/src/sim_capability.rs`

- **USER_ROOT 帯を `0x1000_0000..` に移し、部屋容量を Manifest から導出（継続 + 新規・x009b）** `+2`
  > DoS 天井 32、幅は Manifest 連続 entries。5 人超で pin を上げなくてよくなった。
  > 対象ファイル: `protocol/src/lib.rs`

- **`validate_room_id` の統一と DemoScene 共有テストベクタ（継続）** `+2`
  > 対象ファイル: `protocol/testdata/demo_scene_seal.json`

### client/shared-sim

- **Zero-Base の SoA `EpochSheet`（世代付きハンドル・readiness ビットマスク・dirty / sleep）と Flat LBVH（morton）を `forbid(unsafe_code)` で実装（新規・x008e〜x0093）** `+3`
  > `tape.rs` / `topology.rs` / `spatial.rs` / `xpbd.rs` に分かれ、`XpbdWorld` は prev/curr の二重シートで Verlet を回す（`xpbd.rs` L1-46）。グラフ彩色でバッチ内の端点を素にして並列化の下地を作っている（L93-120）。データ指向設計の方向性はプロジェクト規則と一致する（ただし確保の漏れは weaknesses 参照）。
  > 対象ファイル: `client/shared-sim/src/tape.rs`, `client/shared-sim/src/xpbd.rs`, `client/shared-sim/src/spatial.rs`

- **`FrameBridge` / `EpochSnapshotPair` で host と client が同じ二面スナップショット契約を使う（新規・x008f）** `+1`
  > 対象ファイル: `client/shared-sim/src/bridge.rs`

---

## 技術評価層 — ホストユーザー端末

### client/host-server（Untrusted 計算ホスト）

- **credential worker と「失効で配信停止、一時失敗では殺さない」（継続）** `+4`
  > 対象ファイル: `client/host-server/src/room_token/credential_worker.rs`

- **Zenoh put を network worker に分離、物理はジョブ投入のみ（継続）** `+3`
  > 対象ファイル: `client/host-server/src/network_worker.rs`

- **bind で `cnf.spk == session_pub` を照合、Intent セッション表、Presence 解放の三重化（継続）** `+9`
  > 対象ファイル: `client/host-server/src/session.rs`, `client/host-server/src/presence.rs`

- **Intent / bind rate limit のキーを Ed25519 検証済み `session_pub` に（継続 + 新規・x009c）** `+4`
  > 前回 -1 の「自己申告 uhlc ID」を、署名検証済みの `session_pub` 起点 + soft-global（成功 JWT で返却）に置き換えた。
  > 対象ファイル: `client/host-server/src/intent_rate.rs`

- **`room_token/` 7 ファイル分割（継続）と HTTP PoP クライアント署名（新規・x0081）** `+3`
  > `http_pop_client.rs` が world と同じ canonical bytes で署名する。
  > 対象ファイル: `client/host-server/src/room_token/http_pop_client.rs`

- **inventory を tick 外 worker で処理し、Dual-auth（owner_sub or host）→ seq 検査 → rate → commit の順（継続 + 新規・x0098）** `+3`
  > AEAD 開封 → 署名検証 → `evaluate_control`（未 bind / 非オーナーは共有 rate に到達しない）→ `rate.allow` → `commit_control`（`inventory_spawn.rs` L228-286）。owner は guest Token + Intent bind のままで操作でき、前回 -1 の「take-out UI が host Token 前提」を解消。
  > 対象ファイル: `client/host-server/src/inventory_spawn.rs`, `client/host-server/src/inventory_despawn.rs`

- **JWKS SWR・move クランプ・snapshot 純デルタ・60/30Hz 分離・入力ゲート順・Token 検証の厳格さ・poison 致命扱い（継続）** `+15`
  > 対象ファイル: `client/host-server/src/room_token/jwks.rs`, `client/host-server/src/finite.rs`, `client/host-server/src/room_token/verify.rs`

- **`HostLoop::tick` を切り出し、`main()` を配線だけに（新規・x0099）** `+2`
  > `fn main()` は L430〜約 330 行の起動配線になり、1 tick の可変状態は `HostLoop`（`host_loop.rs` L172-252）が持つ。前回 -1 の一枚岩を解消し、tick をテスト可能にした（`host_loop.rs` に 3 テスト）。
  > 対象ファイル: `client/host-server/src/host_loop.rs`, `client/host-server/src/main.rs`

- **配信 pose `Vec` を `PoseScratchPool` で再利用（新規・x0095）** `+1`
  > acquire → submit → coalesce / worker で返却（`network_worker.rs` L10-17, L143-165）。前回 -1 の「配信 tick ごとの Vec 2 本」を解消。
  > 対象ファイル: `client/host-server/src/network_worker.rs`

- **Room AEAD の epoch 単調採用と prev grace（新規・x007e）** `+2`
  > `apply_token_key` は epoch 後退を無視し、同 epoch で鍵バイトが変われば採用、前進時だけ prev に退避（`room_aead.rs` L73-145）。seal は常に current、open は current → prev。credential worker は 15 秒で host Token を取り直し、epoch 前進時は Manifest を新 epoch で再 put する（`credential_worker.rs` L30-32, L312-317）。
  > 対象ファイル: `client/host-server/src/room_aead.rs`, `client/host-server/src/room_token/credential_worker.rs`

- **world 定義の scene から XPBD ワールドを構築（継続・置換）** `+2`
  > `PhysicsWorld::from_entries` が SceneManifest から地面・mesh ball・箱・user_root を組む（`sim.rs` L1-13）。前回の Rapier 構築 +3 から、機能後退（weaknesses 参照）を踏まえて +2 に下げた。
  > 対象ファイル: `client/host-server/src/sim.rs`

### client/app（体感・描画・UI エンドポイント）

- **検証失敗は discard・退室は明示 Leave のみ、XR 描画を wgpu SceneGpu に統合（継続）** `+8`
  > 対象ファイル: `client/app/src/bus/manifest.rs`, `client/app/src/surface/xr/wgpu_session.rs`

- **信頼ゲートを `client/trust` に切り出し GUI と headless probe で共有（継続）** `+3`
  > 対象ファイル: `client/trust/src/gate.rs`

- **gen/rev 両ロック判定・Frame / Presence 署名と seal + attestation チェーン・system_ui の OpenXR 非依存・実機コントローラ描画（継続）** `+12`
  > 対象ファイル: `client/app/src/bus/`, `client/app/src/system_ui/facet.rs`

- **`main.rs` 分割（182 行）と Desktop / XR hot switch（継続）** `+4`
  > 対象ファイル: `client/app/src/main.rs`, `client/app/src/surface/hotswitch.rs`

- **placeholder + timeout・client JWKS soft/hard・80% 自動更新・surface 自動選択・デコード前 BLAKE3 照合（継続）** `+10`
  > 対象ファイル: `client/app/src/assets_fetch.rs`, `client/app/src/world_token.rs`

- **ToS 同意・mTLS 鍵 DACL・ワーカー HTTP・ワールド作成・Leave 順序・サブリレー化しない（継続）** `+6`
  > 対象ファイル: `client/app/src/auth.rs`, `client/zenoh-connect/src/allocate.rs`

- **HostSupervisor（Job Object / PDEATHSIG / exit latch / ready 待ちを無ロック）（新規・x007c 系 / #144）** `+3`
  > launcher を廃し、client を親にした。Windows は `KILL_ON_JOB_CLOSE`、Linux は `PR_SET_PDEATHSIG`、ready ファイルは IPC bind 後、Exit は「一度立てたら消えない latch」で Start の HTTP 中に Exit されても後から復活しない（`host_supervisor.rs` L1-33, L52-96）。状態機械（Idle / Reserved / Starting / Running）を短時間ロックで回し、ready 待ちは無ロックで cancel 可能。プロセス境界の設計としては同種 OSS（Steam のランチャ子プロセス管理等）と比べても丁寧。※ 認証の受け渡しは weaknesses 参照。
  > 対象ファイル: `client/app/src/host_supervisor.rs`

- **GLB デコード予算（入力・展開・頂点・index・pixel）（新規・x0085）** `+2`
  > `AssetDecodeBudget`（`asset_decode_budget.rs` L20-28）を buffer 宣言長→実長→頂点→index→画像の順に課金（`mesh.rs` L203-241）。テストに爆弾 fixture と各上限ゼロ化がある（L727-877）。
  > 対象ファイル: `client/app/src/asset_decode_budget.rs`, `client/app/src/mesh.rs`

- **guest の epoch probe と AEAD 失敗 nudge、同 epoch では jti を回さない（新規・x007e）** `+1`
  > `room.rs` L7-20。Presence reclaim を壊さない配慮。
  > 対象ファイル: `client/app/src/room.rs`

- **Epoch ミラー → SceneGpu bind / draw、eye display TRS（新規・x0090〜x0092）** `+2`
  > `epoch_view.rs` / `epoch_mirror.rs` で shared-sim のシートを描画側に写す。
  > 対象ファイル: `client/app/src/epoch_view.rs`, `client/app/src/epoch_mirror.rs`

- **Worlds catalog / Host 開始 / handover / takeover の UI 導線（新規・x007c / x0080）** `+1`
  > 対象ファイル: `client/app/src/world_api.rs`

### launcher/

（git 管理から削除済み・評価対象外。退役の完了はプロジェクト全体設計で +1。）

---

## 技術評価層 — 転送・公開エッジ

### relay-server / edge / relay-agent

- **zenohd ACL の非対称化・relay-agent の bounded 化・WAN の TLS + mTLS + deny + role CN（継続）** `+9`
  > 対象ファイル: `relay-server/config.wan.json5`, `relay-agent/src/lib.rs`

- **Allocation 検証・`/internal/*` 404・Tunnel 手順・`zenoh-connect` 共通化・設定分離・WAN runbook・「zenohd は Token を見ない」明記・`TRUSTED_PROXIES`（継続）** `+12`
  > 対象ファイル: `edge/cloudflared/`, `client/zenoh-connect/`

- **relay-agent を CSR 専用にし、非ループバック待受を起動前と bind 後の 2 回で拒否（新規・x0083）** `+2`
  > `csr_pem` 必須（missing / blank / null を区別して 400）（`lib.rs` L86-88, L719-724）、`AllocateResponse` に `key_pem` が無いことをテストで固定（L1737-1738）。`localhost` の hosts 改ざんも bind 後の `ensure_bound_loopback` で拒否（L258-277, L319）。
  > 対象ファイル: `relay-agent/src/lib.rs`

- **Allocation の主体別 throughput（短窓 + 長窓・後段拒否でロールバック・LRU 上限）（新規・x0088）** `+1`
  > 対象ファイル: `relay-agent/src/throughput.rs`

---

## 横断評価層

### テスト戦略

- **WAN 実 host 一往復 E2E（host Accept / Frame・no-cert・guest sim put 非配送・旧 epoch・ワイルドカード不可読）（継続 + 新規・x0087）** `+5`
  > `.scripts/smoke-compose-wan-e2e.sh` L2, L263、`e2e-compose.yml` L96-118。前回の「synthetic Manifest put 止まり」を実 host の credential worker〜sealed Frame まで延ばし、新しい保証（epoch）を否定系で確認する完了条件を守った。
  > 対象ファイル: `.scripts/smoke-compose-wan-e2e.sh`, `.github/workflows/e2e-compose.yml`

- **cargo-fuzz・ヘルメティック Compose E2E・auth の厚いテスト（当日 133 件）（継続）** `+9`
  > 対象ファイル: `.github/workflows/host-fuzz-ci.yml`, `docker-compose.e2e.yml`

- **Rust ユニットテストの厚み（当日 client workspace 447・protocol 73・relay-agent 32 件 PASS）（継続・増加）** `+4`
  > 前回 275 / 67 / 16 から増加。shared-sim の XPBD・LBVH・bridge にもテストがある。
  > 対象ファイル: `client/`, `protocol/`, `relay-agent/`

- **relay-agent のテスト 16 → 32 件（CSR-only・loopback・throughput）（新規）** `+2`
  > 対象ファイル: `relay-agent/src/lib.rs`, `relay-agent/src/throughput.rs`

- **adversarial E2E・汚染パス隔離テスト・host Token 否定系・Rust / Elixir 共有ベクタ・JWKS 時計注入（継続）** `+8`
  > 対象ファイル: `client/e2e-probe/`, `protocol/testdata/`

- **assets テストで reaper / GC を起動しない hermetic 化（新規・x0086）** `+1`
  > `config/test.exs` L20-22。当日の再実行で 191 件 0 failures。
  > 対象ファイル: `assets-server/config/test.exs`

- **world のテスト 116 → 162 件（scene CAS・定期ローテ・deploy baseline・lease CAS）（新規）** `+1`
  > `rooms_test.exs` L67-104 等。
  > 対象ファイル: `world-server/test/`

### 可観測性

- **6 プロセスの loopback Prometheus・Room Token `jti` 相関・auth 構造化ログ・host 周期カウンタ（継続）** `+9`
  > room token 発行ログに `room_key_epoch` と `state` が加わった（`room_controller.ex` L268-272）。
  > 対象ファイル: `world-server/lib/world/metrics.ex`, `client/host-server/src/metrics.rs`

- **auth / world / assets の 3 サービスで `/ready` を liveness と分離（新規・x008b）** `+1`
  > 対象ファイル: `world-server/lib/world/ready.ex`, `auth-server/lib/auth/ready.ex`

### エラーハンドリング

- **anyhow + context / タグ付きタプル・ストレージ障害と不存在の分離・応答から `inspect` 除去・relay の lock `map_err`（継続）** `+6`
  > 対象ファイル: `world-server/lib/world_web/controllers/room_controller.ex`

- **CAS 系の失敗を 409（`scene_revision_conflict` / `host_lease_conflict` / `takeover_conflict`）と 503 / 404 に意味で分けた（新規）** `+1`
  > 再試行してよい失敗（409）と、してはいけない失敗を呼び出し側が区別できる（`room_controller.ex` L681-685, L848-851）。
  > 対象ファイル: `world-server/lib/world_web/controllers/room_controller.ex`

### 変更容易性・保守性

- **モジュールごとの Guarantees / Non-guarantees・`shared/alchemy_*` 共有・`alchemy_auth_client`・shared-sim の単一正本・protocol ゲート・`undocumented_unsafe_blocks = "deny"`（継続）** `+13`
  > 新規モジュール（`http_pop.ex`・`room_key_grace.ex`・`host_supervisor.rs`・`xpbd.rs` 等）もすべて冒頭に保証 / 非保証を書く運用が守られている。
  > 対象ファイル: `client/Cargo.toml`（L17-19）, `shared/`

### 開発者体験（DX）

- **README → compose → cargo run、`ci.sh` 単一入口・dev / release 2 ステージ Docker・prod compose config・rust-cache（継続）** `+7`
  > 対象ファイル: `.scripts/ci.sh`, `README.md`

- **`ci.sh all` が relay-agent に加え `shared/alchemy_*` 3 パッケージも回す（新規・x0089）** `+2`
  > `ci.sh` L147-228。前回の「shared 未収録」を解消。
  > 対象ファイル: `.scripts/ci.sh`

- **Windows ネイティブ（Git Bash + Docker Desktop）で完走（継続・当日 assets のみフレーク）** `+1`
  > 対象ファイル: `.scripts/ci.sh`

### セキュリティ・配布可能性

- **Untrusted host 前提の信頼チェーン（seal + attestation + gen/rev + discard）に、部屋 AEAD・短命 mTLS・制御面署名・HTTP PoP・鍵 epoch が揃った（継続 + 新規）** `+5`
  > 前回の「最後の 1 ピース（Token の運び方と PoP）」が埋まった。Matrix / MLS 系と比べても、共有鍵モデルの範囲では筋の通った多層防御。
  > 対象ファイル: `.workspace/0_doc/architecture/trust-boundary.md`

- **依存脆弱性管理（cargo deny ×2 lock・hex.audit）・入口防御の順序・release から既定資格情報除去・`host_control` 既定 OFF・`PUBLISH_ADDR` 既定 loopback（継続）** `+9`
  > 当日 cargo deny（client / relay-agent）PASS、hex.audit「No retired or security advisory packages」。
  > 対象ファイル: `.github/workflows/rust-ci.yml`

- **Windows x64 テスター ZIP をタグ push → pack → pre-release まで自動化（新規）** `+2`
  > `windows-release.yml`（PowerShell のみ・バージョン書式検証・pre-release / latest=false）。配布物に auth / world / assets / zenohd を含めない境界も明記。
  > 対象ファイル: `.github/workflows/windows-release.yml`, `.scripts/pack-windows-zip.ps1`

- **HostSupervisor ↔ host を loopback IPC + セッショントークンに限定（新規）** `+1`
  > 対象ファイル: `protocol/src/host_ipc.rs`, `client/host-server/src/host_ipc.rs`

### プロジェクト全体設計

- **評価 → 改善計画 → スライス → PR の自己改善サイクルが、前回 P0 4 件・P1 5 件・P2 14 件の大半を 1 週間でコードに落とした（継続・成果）** `+4`
  > x0081〜x009c の 26 スライス。各スライスが保証表と否定系テストを完了条件に含める運用も維持。
  > 対象ファイル: `.workspace/roadmap/slices/`

- **trust-boundary.md の継続保守（x0087 の保証を表に追記）（継続）** `+4`
  > 対象ファイル: `.workspace/0_doc/architecture/trust-boundary.md`（L96）

- **architecture/*.md の「保証する／しない」表とコードの一致（継続・Rapier 表記など一部遅れ）** `+3`
  > 対象ファイル: `.workspace/0_doc/architecture/`

- **4 階層マルチライセンスと注入境界・inventory ladder のスライス化・旧 Elixir 権威からの転換（継続）** `+6`
  > 対象ファイル: `LICENSE`, `TRADEMARKS.md`

- **改善計画にクローズ状況を追記する運用・パスフィルタ CI（継続）** `+2`
  > 対象ファイル: `.github/workflows/`

- **launcher を中途半端に残さず削除し、責務を HostSupervisor に移した（新規）** `+1`
  > 2334 行削除・`launcher.md` に「廃止」明記。ただし README / release workflow に残骸あり（weaknesses）。
  > 対象ファイル: `.workspace/0_doc/architecture/launcher.md`, `client/app/src/host_supervisor.rs`
