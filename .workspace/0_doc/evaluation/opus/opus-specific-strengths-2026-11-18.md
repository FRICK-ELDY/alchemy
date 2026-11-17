# Alchemy — プラス点詳細（第1評価者 Claude Opus 5.5 / 2026-11-18）

> 評価対象コミット: `57ca452`（main・PR #247 / x00d6 直後）。前回（自系統）`8833821` から 135 コミット（x009d〜x00d6 の 58 スライス）。
> 「継続」項目は、前回根拠ファイルについて `git diff 8833821 HEAD` で変更有無を確かめ、変更があったファイルは当日再読した（例: auth-server は 7 ファイル・+159 行のみ）。「新規」「格上げ」項目はすべて当日コードを読んで確認した。
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
| 運営側コア | auth-server | +27 |
| 運営側コア | assets-server | +41 |
| 運営側コア | world-server | +70 |
| 共通契約 | protocol/ | +48 |
| 共通契約 | client/shared-sim | +11 |
| ホスト端末 | client/host-server | +60 |
| ホスト端末 | client/app（HostSupervisor 含む） | +67 |
| ホスト端末 | launcher/ | 対象外（退役） |
| 転送・エッジ | relay-server / edge / relay-agent | +26 |
| 横断 | テスト戦略 | +34 |
| 横断 | 可観測性 | +10 |
| 横断 | エラーハンドリング | +9 |
| 横断 | 変更容易性・保守性 | +14 |
| 横断 | 開発者体験（DX） | +10 |
| 横断 | セキュリティ・配布可能性 | +18 |
| 横断 | プロジェクト全体設計 | +22 |
| | **合計** | **+467** |

---

## 技術評価層 — 運営側コア

### auth-server（アイデンティティ SSoT）

- **Refresh Token のファミリー回転・再利用検知・行ロック付き回転（継続）** `+4`
  > Ory Kratos / Keycloak の最小構成と比べても遜色ない。根拠コードは前回から不変。
  > 対象ファイル: `auth-server/lib/auth/`

- **RS256 + `kid` + 検証鍵ローテ、署名鍵を `:persistent_term` に置き未準備時 fail-closed（継続）** `+4`
  > 対象ファイル: `auth-server/lib/auth/token/keys.ex`

- **多軸 rate limit（IP × 識別子 × トークンファミリー）・Social mutate の両軸 RL・Tunnel 背後 IP 解決の共有（継続・記述訂正）** `+6`
  > `shared/alchemy_client_ip` / `alchemy_rate_limit` を **auth と world** で共有する（前回「assets でも共有」と書いたのは誤り。assets は未導入で、weaknesses の -2 として計上）。当日 shared 3 パッケージ 25 テスト PASS。
  > 対象ファイル: `auth-server/lib/auth_web/`, `shared/alchemy_client_ip/`, `shared/alchemy_rate_limit/`

- **アカウント列挙・タイミング攻撃への配慮、メール確認 / パスワードリセットのトランザクション設計（継続）** `+4`
  > 対象ファイル: `auth-server/lib/auth/`

- **失敗分類ログ・`access_jti` metadata・S2S 内部トークンの prod fail-closed・パイプライン分離・loopback metrics（継続）** `+5`
  > 対象ファイル: `auth-server/lib/auth_web/router.ex`, `auth-server/lib/auth_web/plugs/`

- **Follow / Friendship の DB 制約付きソーシャルグラフ（継続）** `+2`
  > 対象ファイル: `auth-server/lib/auth/`

- **`/ready`（Repo・Keys）を liveness と分離（継続）** `+1`
  > 対象ファイル: `auth-server/lib/auth/ready.ex`

- **関係解除を world に通知する S2S フック（新規・x00a0）** `+1`
  > `Auth.WorldNotify.relation_changed/3` が unfriend / unfollow 後に `POST /internal/v1/relation_changed` を内部トークン付きで投げる。Social API の応答を world の遅延で止めないよう `Task.Supervisor` に逃がし、750ms で打ち切る（`world_notify.ex` L14-30, L56-80）。本番は `WORLD_INTERNAL_URL` が無いと起動しない（`config/runtime.exs` L88-96）。再試行が無い点は weaknesses 参照。
  > 対象ファイル: `auth-server/lib/auth/world_notify.ex`

### assets-server（CAS・Quota・門番）

- **CAS 物理から access を剥がし、GET を常時 Grant / PlaceRead で門番（継続）** `+7`
  > 対象ファイル: `assets-server/lib/assets/blobs.ex`

- **reserved reaper + orphan CAS GC の quarantine → 再確認 → discard / restore（継続）** `+3`
  > 対象ファイル: `assets-server/lib/assets/quota/reserved_reaper.ex`, `assets-server/lib/assets/cas/orphan_gc.ex`

- **Quota 予約 → TX 外 CAS put → 確定、条件付き UPDATE による原子更新（継続）** `+5`
  > 対象ファイル: `assets-server/lib/assets/blobs.ex`

- **検疫を `Assets.Quarantine.Worker` Behaviour + `public_reason/1` に分離（継続・ライセンス境界準拠）** `+2`
  > pgvector 等の Proprietary は OSS ツリーに無い。
  > 対象ファイル: `assets-server/lib/assets/quarantine/`

- **BLAKE3 を常駐 Port で計算し `PartitionSupervisor` で N 本プール化。VCS object 検証も同形（継続 + 拡張）** `+4`
  > x00c9 の `Assets.VcsObject.Worker` が Blake3 と同じ framing / reopen / 全パーティション ready を踏襲した（`vcs_object/worker.ex` L1-11）。新しい Port を足しても運用面の形が変わらない。
  > 対象ファイル: `assets-server/lib/assets/blake3/worker.ex`, `assets-server/lib/assets/vcs_object/worker.ex`

- **メタと不変バイナリの分離・CAS backend behaviour・s3 時 302（継続）** `+6`
  > 対象ファイル: `assets-server/lib/assets/cas/`

- **Drive / VCS 薄皮で take_out・put_back の全 blake3 に Grant を要求（継続）** `+1`
  > 対象ファイル: `assets-server/lib/assets/drive.ex`

- **Origin Cert を v2（commit / release の 2 形式・Ed25519・オフライン検証）に（格上げ・x00ca）** `+3`
  > 署名対象を `v2|commit|{commit_hash}|{author_sub}|{issued_at_ms}|{kid}` と `v2|release|{repo_id}|{tag_name}|{commit_hash}|{signer_sub}|{issued_at_ms}|{kid}` に固定し（`vcs/cert.ex` L7-8）、各フィールドに `|` を含めないことを検証してから連結する（L304-307, L418-429）。公開鍵だけで検証できる `verify_offline_authorship` / `verify_offline_release`（L286-314, L329-386）を用意し、連合先が assets に問い合わせずに検証できる形にした。過去鍵を `kid` で引かない点は moduledoc の非保証（L47-50）に明記。前回 v1（+2）からの格上げ。
  > 対象ファイル: `assets-server/lib/assets/vcs/cert.ex`

- **GET を `application/octet-stream` + `nosniff` 固定（継続）** `+2`
  > 対象ファイル: `assets-server/lib/assets_web/controllers/blob_controller.ex`

- **生 body 受信・local CAS の tmp → rename・テスト環境の CAS env 無視（継続）** `+3`
  > 対象ファイル: `assets-server/lib/assets/cas/local.ex`, `assets-server/config/test.exs`

- **Spatial VCS の push を「owner のみ・全 object を Rust で厳格検証・作者一致・子 Grant・ref CAS + fast-forward・単一 TX」で実装（新規・x00c9〜x00cd）** `+4`
  > `Assets.Vcs.Push` は件数上限（新規 commit 256・object 1 MiB・hash 512、`vcs/push.ex` L32-34, L144-176）を先に課し、各 object を Port 経由の `alchemy_protocol::vcs_object` で正規形デコードし直してから、commit の `author_sub` が push 主体と一致すること（L99）、tree の子 blob を push 主体が Grant で持つこと、ref 更新が fast-forward であること（L105）を確かめ、ref CAS まで 1 TX で行う。git の receive-pack + fsck + ref ロックに相当する流れを、「中身はサーバーが再検証する・権利は Grant で見る」という Alchemy の原則で書き直している。repo の可視性（RepoRead、x00cb）も `vcs/repo.ex` / `repo_blob.ex` で分離。clone / fetch / branch UI は非保証として正直に残している。
  > 対象ファイル: `assets-server/lib/assets/vcs/push.ex`, `assets-server/lib/assets_web/controllers/vcs_controller.ex`, `assets-server/lib/assets/vcs/repo.ex`

- **SpawnTicket を発行側で Grant 照会してから RS256 で署名（新規・x00b5）** `+1`
  > blake3 ごとに発行者が Grant を持つことを確かめてから、`typ = spawn_ticket`・`aud = alchemy-world-spawn`・room 束縛の短命 JWT を出す。world 側の消費は world-server 項で評価。
  > 対象ファイル: `assets-server/lib/assets/spawn_ticket.ex`, `assets-server/lib/assets_web/controllers/spawn_ticket_controller.ex`

### world-server（案内所・門番・監査）

- **ManifestSeal を DB の scene + 検証済み attestation から world 自身が計算（継続）** `+4`
  > 対象ファイル: `world-server/lib/world_web/controllers/room_controller.ex`

- **CSR から短命 mTLS を署名し CN をサーバーが強制（継続）** `+3`
  > 対象ファイル: `world-server/lib/world/mtls_issuer.ex`

- **PlaceRead の `exp` を検証済み seal の絶対 `exp` に一致（継続）** `+3`
  > 対象ファイル: `world-server/lib/world/place_hashes.ex`

- **HostAttestation / ManifestSeal の typ・aud 分離 RS256 と host_generation（継続）** `+3`
  > 対象ファイル: `world-server/lib/world/host_attestation.ex`, `world-server/lib/world/manifest_seal.ex`

- **host 専用 HTTP に Ed25519 PoP（`cnf.spk`）を必須化（継続）** `+4`
  > 新しい host 経路（overlay entries・scene save・handover accept 系）もすべて `require_host_token` → `require_http_pop` の順を通る。
  > 対象ファイル: `world-server/lib/world/http_pop.ex`, `world-server/lib/world_web/controllers/room_controller.ex`

- **Room 鍵 epoch（CAS ローテ・host 限定 grace・定期ローテ・deploy baseline）（継続）** `+4`
  > x00d6 で `rooms/room_key.ex` に分離された。
  > 対象ファイル: `world-server/lib/world/rooms/room_key.ex`, `world-server/lib/world/rooms/room_key_grace.ex`

- **canonical scene の revision CAS（継続）** `+2`
  > x009e で canonical × stride + overlay の packed revision に拡張（下の HostDelegation 項）。
  > 対象ファイル: `world-server/lib/world/rooms/scene.ex`

- **host-token 系 rate limit の jti 軸を Bearer claims から取る（継続）** `+2`
  > 対象ファイル: `world-server/lib/world_web/plugs/rate_limit.ex`

- **永続オーナーと現計算ホストを分離し、lease を CAS で管理（継続）** `+3`
  > 対象ファイル: `world-server/lib/world/rooms/lease.ex`

- **Room 上限を quota 行 + room insert の単一 TX に（継続）** `+1`
  > 対象ファイル: `world-server/lib/world/rooms/owner_quota.ex`

- **Worlds catalog が秘密を返さず、owned を global top-N で落とさない（継続）** `+2`
  > 対象ファイル: `world-server/lib/world/rooms.ex`

- **seal-hash Port のプール化と `/ready`（継続）** `+2`
  > 対象ファイル: `world-server/lib/world/manifest_seal_hash/worker.ex`

- **Room 鍵の CSPRNG 発行と Allocation JWT 非搭載（継続）** `+2`
  > 対象ファイル: `world-server/lib/world_web/controllers/room_controller.ex`

- **`:user_auth` 後段の IP / user 両軸 rate limit（継続・handover accept / eligible_hosts / rotate にも適用）** `+2`
  > `action_for/1` に handover accept・eligible_hosts・room_key rotate の節が加わった（`plugs/rate_limit.ex` L77-106）。
  > 対象ファイル: `world-server/lib/world_web/plugs/rate_limit.ex`

- **Room Token に `cnf.spk` を埋め込み、発行時に検証（継続）** `+2`
  > 対象ファイル: `world-server/lib/world/token.ex`

- **Ash + SQLite 永続化とストレージ障害の 503 分離、公開範囲 ACL の S2S fail-closed、Room Token claim + jti metadata、auth JWT `sub` 転写（継続）** `+8`
  > 対象ファイル: `world-server/lib/world/`, `world-server/lib/world/auth_access.ex`

- **Verifier を `AuditEngine` Behaviour の注入点に留め、Zenohex を adapter 1 モジュールに隔離（継続）** `+2`
  > 対象ファイル: `world-server/lib/world/verifier/`

- **`wire_cap` と Allocation Token の分離・`lan` fail-closed・匿名 Token 既定 OFF（継続）** `+3`
  > 対象ファイル: `world-server/lib/world/allocation_token.ex`, `world-server/lib/world/room_access.ex`

- **HostDelegation: canonical は owner ∪ editor、takeover host は session overlay だけ、takeover 自体も owner ∪ eligible に限定（新規・x009d / x009e）** `+4`
  > 前回の最大欠陥（-3〜-4: takeover した第三者がオーナーの canonical scene を書き換えられる）を、権限モデルごと作り直して閉じた。canonical の append / delete / save は `ensure_active_host` に加えて `Rooms.ensure_scene_writer/2`（owner ∪ `editor_subs`、`rooms.ex` L223-245）を通す（`room_controller.ex` L637, L671, L706）。takeover host が置けるのは `session_overlay` だけで、host 交代や dormant で overlay を捨てて revision を進める（`rooms/internal.ex` L43-53）。canonical と overlay は 1 つの packed revision で CAS するので、片方だけ進む中間状態が無い。takeover できるのも owner ∪ `eligible_host_subs`（`PUT …/eligible_hosts`）に限った。否定系テストが controller（`room_controller_test.exs` L2097-2311 の `scene_writer_forbidden` 4 箇所）・`scene_writer_test.exs`・`session_overlay_test.exs` L45 に入っている。Resonite の「Host でも保存先の所有権が無ければワールドを上書きできない」と同じ線引きを、world 側の権限表として明示した。
  > 対象ファイル: `world-server/lib/world/rooms.ex`, `world-server/lib/world/rooms/internal.ex`, `world-server/lib/world/rooms/lease.ex`, `world-server/lib/world_web/controllers/room_controller.ex`

- **handover を「指名 → 候補が accept」の二段階 CAS に（新規・x00a1）** `+2`
  > 退出する host は Presence の相手を**候補**として指名するだけで、lease は動かない。候補側の client が `POST …/handover/accept`（`router.ex` L30）を呼んで初めて active host になる（client 側は `session_flow.rs` L1408-1442 と `HostStartKind::Nominated` L1752）。同意していない人に 5 分の lease が付く前回の問題（-2）を閉じた。
  > 対象ファイル: `world-server/lib/world/rooms/lease.ex`, `client/app/src/surface/desktop/session_flow.rs`

- **SceneBudget（entries 4096・JSON 1 MiB）を全 scene CAS 経路に適用（新規・x00a2）** `+2`
  > `World.SceneBudget` を `cas_set_scene` / `cas_set_overlay` / `cas_set_scene_and_cleared_overlay` のすべてで encode 時に課し、合成後（canonical + overlay）も検査する。上限値は protocol の `MAX_MANIFEST_BYTES` と同じ数字で（`decode_limits.rs` L52-53 のコメント）、world が受理したものを client が decode 上限で落とす、という層間の食い違いを防いでいる。
  > 対象ファイル: `world-server/lib/world/scene_budget.ex`, `world-server/lib/world/rooms/scene.ex`

- **HTTP PoP の nonce を 1 回限りに（新規・x00d4）** `+1`
  > `{jti, nonce}` を ETS に `insert_new` し、`exp = pop_ts + skew` で掃除する。署名対象も `alchemy/pop/v1|{METHOD}|{path}|{ts}|{nonce}|{raw_body}` に拡張し protocol と lockstep。world 再起動での揮発は trust-boundary に非保証として明記。
  > 対象ファイル: `world-server/lib/world/http_pop.ex`, `world-server/lib/world/http_pop/nonce.ex`

- **owner の鍵ローテ HTTP と、関係解除時の自動ローテ（新規・x00a0）** `+2`
  > `POST …/room_key/rotate`（owner のみ・RL 付き）と `POST /internal/v1/relation_changed` → `rotate_for_relation_change`。friends 部屋でフレンドを外すと、旧鍵で新 epoch が読めなくなる。拒否リストが無い点は weaknesses 参照。
  > 対象ファイル: `world-server/lib/world/rooms/room_key.ex`, `world-server/lib/world_web/controllers/relation_controller.ex`

- **SpawnTicket を world が 1 回だけ消費し、scene 書き込みに資産の権利を結び付けた（新規・x00b5）** `+3`
  > `World.SpawnTicket` は assets 発行の JWT を `typ` / `aud` / room 一致 / blake3 が許可リストに含まれること / 発行先が現在の参加者であること（`RoomParticipants.participant?`）で検証し、`(jti, blake3)` を一回限りで消費する。scene / overlay の CAS が競合で負けたときだけ消費を戻す。前回 -1 の「scene append が Grant を見ない」を、world が assets の DB を覗かずに済む**サービス間の権限証票**で閉じた。overlay / scene の両方で「ticket 無し → 403 `spawn_ticket_required`」の否定系テストがある（`room_controller_test.exs` L2821-2898）。
  > 対象ファイル: `world-server/lib/world/spawn_ticket.ex`, `world-server/lib/world/room_participants.ex`

- **catalog の rate limit と、epoch 専用の軽量 API（新規・x00d2）** `+1`
  > `GET /rooms` は `room_catalog` バケット、`GET …/room_key_epoch`（`router.ex` L51-52）は Room Token Bearer だけで PoP 無しの軽量 probe。host / guest は同じ epoch なら Room Token を再発行しない（`client/app/src/room.rs` L20, L707-729、`credential_worker.rs` L5, L181）。
  > 対象ファイル: `world-server/lib/world_web/plugs/rate_limit.ex`, `world-server/lib/world_web/router.ex`

- **`Rooms` をファサード + 責務別モジュールに、エラー → HTTP 変換を `FallbackController` 1 箇所に（新規・x00d6）** `+2`
  > `rooms.ex` は 1734 → 540 行のファサードになり、`Rooms.{Internal, Lease, RoomKey, Scene, Catalog}` に分かれた。各アクションに手書きされていたエラー変換は `fallback_controller.ex` の表（例: L43 `forbidden_scene_writer: {:forbidden, "scene_writer_forbidden"}`）に集約。前回 -1 の一枚岩を閉じた。
  > 対象ファイル: `world-server/lib/world/rooms.ex`, `world-server/lib/world_web/controllers/fallback_controller.ex`

- **overlay → canonical の Save を packed CAS 1 回で（新規・x00b7 / x00b8）** `+1`
  > `POST …/scene/save` が pending の spawn / despawn / parent / SetField を canonical に畳み込み、overlay クリアと同じ UPDATE で行う。host は revision が進んだときだけ scene を差し替えて再封緘する（`host-server/src/edit.rs` L11-13）。rate limit の抜けは weaknesses 参照。
  > 対象ファイル: `world-server/lib/world/rooms/scene.ex`, `world-server/lib/world_web/controllers/room_controller.ex`

---

## 技術評価層 — 共通契約層

### protocol/（ワイヤ契約 SSoT）

- **Room 鍵 AEAD v2（AAD に epoch）と current → prev 開封の純関数（継続）** `+5`
  > 対象ファイル: `protocol/src/wire_aead.rs`

- **制御面署名の canonical bytes（f64 を `to_bits` hex で固定）（継続）** `+2`
  > 対象ファイル: `protocol/src/control_plane.rs`

- **HTTP PoP の署名バイト列を protocol と Elixir で lockstep（継続・nonce 込みで更新）** `+2`
  > 対象ファイル: `protocol/src/http_pop.rs`

- **decode 上限・`(generation, revision)` 単調ゲート・チャネル責務分離・`wire_crypto` 集約・配送クラス契約（継続）** `+15`
  > 対象ファイル: `protocol/src/decode_limits.rs`, `protocol/src/lib.rs`, `protocol/peer-delivery.md`

- **Port framing・`PROTOCOL_PIN`（`0.24.0-x00b8`）・TRS 正規化・`sim_capability`・独立クレート + protoc 検査・キー式ヘルパ（継続）** `+12`
  > 対象ファイル: `protocol/src/lib.rs`（L80）, `protocol/src/sim_capability.rs`

- **USER_ROOT 帯と部屋容量の Manifest 導出（継続）** `+2`
  > 対象ファイル: `protocol/src/lib.rs`

- **`validate_room_id` の統一と共有テストベクタ（継続）** `+2`
  > 対象ファイル: `protocol/testdata/`

- **Spatial VCS object の決定的 CBOR コーデック（新規・x00c8）** `+3`
  > 「同じ論理 Tree / Commit → 同じ正規バイト列 → 同じ blake3」（`vcs_object.rs` L4）を保証し、厳格デコードは非正規 CBOR ヘッド・未知キー・末尾ゴミ・非正規形を拒否する（L5, L133-164）。object 1 MiB・子 4096 の上限（L21-23）を encode / decode の両側で課し、悪意ある長さ宣言でも先に止まる。TRS は f32 LE 40 byte 固定。git の object 形式より厳密（git は zlib 圧縮と緩い tree ソートの歴史的揺れを抱える）で、Elixir 側は同じコードを Port 越しに使うので二重実装の食い違いが起きない。
  > 対象ファイル: `protocol/src/vcs_object.rs`, `protocol/src/bin/vcs_object.rs`

- **Spatial Interest のキー体系（cell / lod・Moore 近傍・LOD 刻み・opt-out）（新規・x00be / x00c7）** `+2`
  > `frame_cell_key` / `moore_neighborhood_cells` / `LOD_CELL_STRIDE = 3` と、`ALCHEMY_ROOM_FRAME` による Room frame への退避を protocol に置き、host・client・ACL が同じキー式を使う。
  > 対象ファイル: `protocol/src/lib.rs`

- **`sim_state.proto` / `edit.proto` と RenderFrame の拡張（`sim_tick`・冗長・`acked_intent_seq`・`scene_rev`・`holder`）を pin 更新つきで（新規・x00af）** `+2`
  > ワイヤ互換を壊す変更を pin で区切る運用が守られている。
  > 対象ファイル: `protocol/proto/`, `protocol/src/lib.rs`

- **新しいワイヤにも decode 上限を先に課す（`WireClass::SimState`・`EditProposal` の AEAD 封筒上限）（新規）** `+1`
  > `MAX_SIM_STATE_BYTES`（`decode_limits.rs` L58, L144）と `wire_aead::check_aead_envelope_bytes(AeadKind::EditProposal, …)`。host の edit レーンは AEAD を開ける**前**に封筒サイズを見る（`host-server/src/edit.rs` L91-95）。
  > 対象ファイル: `protocol/src/decode_limits.rs`, `protocol/src/wire_aead.rs`

### client/shared-sim

- **Zero-Base の SoA `EpochSheet` と Flat LBVH（morton radix）を `forbid(unsafe_code)` で実装（継続）** `+3`
  > 方向性はプロジェクトの DOD 規則どおり。LBVH 構築の AABB 引き当てが O(n²) になっている点は weaknesses 参照。
  > 対象ファイル: `client/shared-sim/src/tape.rs`, `client/shared-sim/src/spatial.rs`

- **`FrameBridge` / `EpochSnapshotPair` で host と client が同じ二面スナップショット契約を使う（継続）** `+1`
  > 対象ファイル: `client/shared-sim/src/bridge.rs`

- **MVCC の HotPhysicsPage（1024 entity / page）・arena・CoW・EntityDirectory と、確保回数上限のテスト（新規・x00a3〜x00a7）** `+3`
  > ページ単位の SoA と CoW で「読み手（配信）を止めずに書き手（物理）が進む」構造を作り、`arena_cow_1k_entities_500_ticks_page_alloc_bounded` が 1k entity × 500 tick で page 確保 ≤ 2 / tick を数えて固定している。前回提案した「カウンタアロケータで確保回数をテスト」を、まず MVCC 経路で実現した。
  > 対象ファイル: `client/shared-sim/src/mvcc/page.rs`, `client/shared-sim/src/mvcc/arena.rs`, `client/shared-sim/src/mvcc/directory.rs`

- **安全な SIMD（`wide`）とスカラー実装のビット一致テスト（新規・x00a5）** `+1`
  > `step_inertia_scalar` / `step_inertia_simd`（`mvcc/inertia.rs` L40, L94）を併置し、`scalar_and_simd_bit_identical`（L252）で決定論を担保。`unsafe` を使わずに SIMD を得る判断は、決定論が要る物理では正しい。
  > 対象ファイル: `client/shared-sim/src/mvcc/inertia.rs`

- **dirty delta コーデックを shared-sim に置き、host と client で共有（新規・x00ae）** `+2`
  > 対象ファイル: `client/shared-sim/src/delta_codec.rs`

- **XPBD の不等式接触（引き付けない非貫通）（新規・x00a8）** `+1`
  > `contact_constraints_from_lbvh_into`（`xpbd.rs` L273-304）が重なり対を不等式拘束にし、`c.is_inequality() && c_err >= 0.0` で引力を生まない（L222）。`contact_never_attracts` / `…_via_lbvh_step`（L760-790）とセンサー球の除外（L874）をテストで固定。
  > 対象ファイル: `client/shared-sim/src/xpbd.rs`

---

## 技術評価層 — ホストユーザー端末

### client/host-server（Untrusted 計算ホスト）

- **credential worker と「失効で配信停止、一時失敗では殺さない」（継続・epoch probe 化）** `+4`
  > ≈15 秒ごとの同期は `GET …/room_key_epoch` を先に見て、同じ epoch なら Token を取り直さない（`credential_worker.rs` L5, L181）。
  > 対象ファイル: `client/host-server/src/room_token/credential_worker.rs`

- **Zenoh put を network worker に分離、物理はジョブ投入のみ（継続）** `+3`
  > 対象ファイル: `client/host-server/src/network_worker.rs`

- **bind で `cnf.spk == session_pub` を照合、Intent セッション表、Presence 解放の三重化（継続）** `+9`
  > 対象ファイル: `client/host-server/src/session.rs`, `client/host-server/src/presence.rs`

- **Intent / bind rate limit のキーを Ed25519 検証済み `session_pub` に（継続）** `+4`
  > 対象ファイル: `client/host-server/src/intent_rate.rs`

- **`room_token/` 分割と HTTP PoP クライアント署名（継続）** `+3`
  > 対象ファイル: `client/host-server/src/room_token/http_pop_client.rs`

- **inventory を tick 外 worker で処理し、Dual-auth → seq → rate → commit の順（継続）** `+3`
  > 対象ファイル: `client/host-server/src/inventory_spawn.rs`

- **JWKS SWR・move クランプ・snapshot 純デルタ・60/30Hz 分離・入力ゲート順・Token 検証の厳格さ・poison 致命扱い（継続）** `+15`
  > 対象ファイル: `client/host-server/src/room_token/jwks.rs`, `client/host-server/src/finite.rs`

- **`HostLoop::tick` の切り出し（継続）** `+2`
  > 対象ファイル: `client/host-server/src/host_loop.rs`

- **配信 pose `Vec` を `PoseScratchPool` で再利用（継続）** `+1`
  > 対象ファイル: `client/host-server/src/network_worker.rs`

- **Room AEAD の epoch 単調採用と prev grace（継続）** `+2`
  > 対象ファイル: `client/host-server/src/room_aead.rs`

- **world 定義の scene から XPBD ワールドを構築し、接触を戻して `rapier3d` を外した（格上げ・x00ad）** `+3`
  > 前回 -2 の「body-body 接触と Grab 回転追従の喪失」を閉じ、使っていない `rapier3d` を依存から外した。ログも `XPBD world ready (backend=… mode=…)`（`main.rs` L1138）に変わった。+2 → +3。
  > 対象ファイル: `client/host-server/src/sim.rs`, `client/host-server/src/main.rs`

- **親からの AUTH_PUSH を ParentBroker のときだけ適用（新規・x009f）** `+1`
  > `apply_parent_auth_push` は ParentBroker モードでのみ受理し、durable な子には適用しない。IPC の `OK` は inbox 受理であって適用完了ではない、という意味の区別も保証表に書いてある。
  > 対象ファイル: `client/host-server/src/room_token/credential_chain.rs`, `client/host-server/src/host_ipc.rs`

- **決定的 edit レーン（AEAD 封筒上限 → AEAD → Ed25519 → Dual-auth → 共有 rate → `control_seq`、Commit は物理適用後）（新規・x00b2〜x00b9）** `+3`
  > `…/edit` は guest が put できる入口なので、AEAD を開く前に封筒サイズを見て（`edit.rs` L91-95）、開封 → 署名 → room 一致 → session 鍵長 → Dual-auth → 共有 control rate（拒否時は refund、L335-359）の順に通す。Spawn / Despawn は HostLoop で物理適用に成功してから tip を進めて accept Commit を出し、inbox が閉じていれば予約を解放して `APPLY` で reject、tip は巻き戻さない（L116-170）。「受理したが反映されていない」状態を外に出さない設計は、Bevy の Command キューや Resonite の同期より一段厳格。`control_seq` が適用前に進む窓は非保証として正直に書いてある（L16-18）。
  > 対象ファイル: `client/host-server/src/edit.rs`, `client/host-server/src/live_edit.rs`

- **30Hz の dirty delta 本線切替と冗長送信（新規・x00b1）** `+2`
  > 対象ファイル: `client/host-server/src/network_worker.rs`

- **`sim_state`（AEAD + 署名・quarantine・`HostBootMode`）の土台（新規・x00b0）** `+1`
  > planned / crash handover で物理状態を引き継ぐ下地。HostSupervisor からはまだ配線されていない（提案参照）。
  > 対象ファイル: `client/host-server/src/sim_state.rs`

- **Spatial Interest の host 側（非空セルだけセル別封緘で soft-put・境界横断 remove・同一 tick 多セル合流・lod ≤ 5Hz・LRU 64）（新規・x00be〜x00c7）** `+3`
  > Room frame の put 失敗は fail-closed（exit 78）、cell / lod は soft-fail と、本線と補助の失敗方針を分けている。帯域を AOI に比例させる仕組みとしては、VRChat / Resonite の「全員に全員」より明確に先を行く。取りこぼし回復の欠如は weaknesses 参照。
  > 対象ファイル: `client/host-server/src/network_worker.rs`

- **`ALCHEMY_SIM_BACKEND` の未知値は起動拒否（新規・x00ad）** `+1`
  > 既定 `EpochXpbd`、未知値は `Err`。設定ミスで黙って別の物理が走ることが無い。
  > 対象ファイル: `client/host-server/src/sim_backend.rs`

### client/app（体感・描画・UI エンドポイント）

- **検証失敗は discard・退室は明示 Leave のみ、XR 描画を wgpu SceneGpu に統合（継続）** `+8`
  > 対象ファイル: `client/app/src/bus/manifest.rs`, `client/app/src/surface/xr/wgpu_session.rs`

- **信頼ゲートを `client/trust` に切り出し GUI と headless probe で共有（継続）** `+3`
  > 対象ファイル: `client/trust/src/gate.rs`

- **gen/rev 両ロック判定・Frame / Presence 署名と seal + attestation チェーン・system_ui の OpenXR 非依存・実機コントローラ描画（継続）** `+12`
  > 対象ファイル: `client/app/src/bus/`, `client/app/src/system_ui/facet.rs`

- **`main.rs` 分割と Desktop / XR hot switch（継続）** `+4`
  > 対象ファイル: `client/app/src/main.rs`

- **placeholder + timeout・client JWKS soft/hard・80% 自動更新・surface 自動選択・デコード前 BLAKE3 照合（継続）** `+10`
  > 対象ファイル: `client/app/src/assets_fetch.rs`, `client/app/src/world_token.rs`

- **ToS 同意・mTLS 鍵 DACL・ワーカー HTTP・ワールド作成・Leave 順序・サブリレー化しない（継続）** `+6`
  > 対象ファイル: `client/app/src/auth.rs`, `client/zenoh-connect/src/allocate.rs`

- **HostSupervisor（Job Object / PDEATHSIG / exit latch / ready 待ちを無ロック）（継続）** `+3`
  > 対象ファイル: `client/app/src/host_supervisor.rs`

- **GLB デコード予算を「ヘッダ寸法 → pixel 課金 → デコード」の順に（格上げ・x00d1）** `+3`
  > 前回 -1 の「pixel 判定がフルデコード後」を、`probe_image_dimensions` → `add_image_pixels` → decode → `ensure_decoded_matches_header`（`mesh.rs` L236-242）で閉じた。ヘッダと実寸の食い違いも拒否する。+2 → +3。
  > 対象ファイル: `client/app/src/mesh.rs`, `client/app/src/asset_decode_budget.rs`

- **guest の epoch probe と AEAD 失敗 nudge（継続・軽量 API 化）** `+1`
  > 対象ファイル: `client/app/src/room.rs`

- **Epoch ミラー → SceneGpu bind / draw（継続）** `+2`
  > 対象ファイル: `client/app/src/epoch_view.rs`, `client/app/src/epoch_mirror.rs`

- **Worlds catalog / Host 開始 / handover / takeover の UI 導線（継続）** `+1`
  > 対象ファイル: `client/app/src/world_api.rs`

- **親 client を資格情報 broker にし、子 host へは短命 access JWT だけを AUTH_PUSH（新規・x009f）** `+3`
  > 前回 -2〜-3 の「GUI 起動 host が約 15 分で Room Token を再発行できなくなる」を閉じた。`refresh_token` は**決して子に渡さず**（`host_supervisor.rs` L30-31）、親が refresh した直後に loopback IPC で access JWT を push し、短い再試行を持つ（L133-185、`room.rs` の refresh スレッドから呼ぶ）。OAuth の「refresh は最も信頼できるプロセスだけが持つ」原則を、プロセス境界でそのまま実装している。
  > 対象ファイル: `client/app/src/host_supervisor.rs`, `client/app/src/room.rs`

- **remember-me の refresh token を OS の資格情報ストアへ（新規・x00cf）** `+2`
  > `keyring` で `alchemy.refresh` サービスに保存し（`refresh_vault.rs` L13, L44-58）、prefs JSON には key id だけを残す。旧形式の平文は移行して消す。OS ストアが使えない環境では明示的に session-only に落ちる（L4, L123-156）。依存は人の承認記録つき（`client/app/Cargo.toml` L41）。
  > 対象ファイル: `client/app/src/refresh_vault.rs`, `client/app/src/operator_prefs.rs`

- **KeyStale を「pending AEAD の二重開封 → Manifest で一括昇格」の無停止切替に（新規・x00d5）** `+2`
  > epoch が進んだとき、新鍵を pending として開封候補に足し、Manifest を新鍵で開けた時点で live に上げる。`bus_generation` の不一致は fail-closed。前回 -1 の「KeyStale は手動 Rejoin」を閉じた。
  > 対象ファイル: `client/app/src/bus/wire.rs`, `client/app/src/scene.rs`

- **Spatial Interest の client 側（Moore ≤ 27 キーの個別購読・追従再購読・exit prune・near 優先で lod を上書きしない）（新規・x00bf〜x00c7）** `+2`
  > 受信予算が Moore 近傍で上限化される（`bus/frame.rs` L44-45）。同一 tick の複数セルを合流し、セル間 upsert の衝突を防ぐ（L23-25, L33-35）。
  > 対象ファイル: `client/app/src/bus/frame.rs`

- **持ち物の手元予測（Keep / Blend / Snap）と sim 時刻基準の jitter クロック（新規・x00aa / x00ab）** `+2`
  > `owned_prediction.rs` は host の権威と比較して Keep（5cm 以内は予測優先）/ Blend / Snap を選び、Grab 拒否は Snap に倒す（L1-31）。`sim_jitter.rs` は host 時刻で 1〜2 フレーム遅らせた再生ヘッドから α を出す（L1-25）。前回提案した「Frame 補間・jitter buffer」の CPU 側を実装した。`interp.wgsl` は naga 検証のみの足場（提案参照）。
  > 対象ファイル: `client/app/src/owned_prediction.rs`, `client/app/src/sim_jitter.rs`

- **Desktop をイベント → コマンドに分割（新規・x00d0）** `+1`
  > `surface/desktop/{session,inventory,hotswitch}_flow.rs` に分かれ、`system_ui` は `UiAction` を返すだけ。
  > 対象ファイル: `client/app/src/surface/desktop/`, `client/app/src/system_ui/mod.rs`

- **Dash Room タブ・Grab ラジアルからの Drive 保存（新規・x00b6 / x00ce）** `+1`
  > 対象ファイル: `client/app/src/system_ui/dash.rs`

- **Spatial Lens（uber SDF・Hierarchy / Inspector）（新規・x00ba〜x00bd）** `+1`
  > 対象ファイル: `client/app/src/spatial_lens/`

### launcher/

（git 管理から削除済み・評価対象外。）

---

## 技術評価層 — 転送・公開エッジ

### relay-server / edge / relay-agent

- **zenohd ACL の非対称化・relay-agent の bounded 化・WAN の TLS + mTLS + deny + role CN（継続）** `+9`
  > 対象ファイル: `relay-server/config.wan.json5`, `relay-agent/src/lib.rs`

- **Allocation 検証・`/internal/*` 404・Tunnel 手順・`zenoh-connect` 共通化・設定分離・WAN runbook・「zenohd は Token を見ない」明記・`TRUSTED_PROXIES`（継続）** `+12`
  > 対象ファイル: `edge/cloudflared/`, `client/zenoh-connect/`

- **relay-agent を CSR 専用にし、非ループバック待受を 2 回拒否（継続）** `+2`
  > 対象ファイル: `relay-agent/src/lib.rs`

- **Allocation の主体別 throughput（継続）** `+1`
  > 対象ファイル: `relay-agent/src/throughput.rs`

- **JWKS 取得を `ureq`（5 秒タイムアウト）に置き換え、lock の `expect` を除去（新規・x00d3）** `+1`
  > 前回 -1 の `curl` 子プロセスを閉じた。残る `Command::new` は openssl による CSR 生成のみ。依存は人の承認記録つき（`relay-agent/Cargo.toml` L23）。
  > 対象ファイル: `relay-agent/src/lib.rs`

- **新しいキー（cell / lod / edit / sim_state）を ACL に明示列挙し、`frame/**` に広げない（新規）** `+1`
  > 本番用と E2E 用の 2 設定を同時に更新し、guest は `sim_state` を購読できず、`edit` は put できる、という非対称もそのまま保った。ワイルドカードで楽をしない判断が良い（E2E が追随していない点は weaknesses 参照）。
  > 対象ファイル: `relay-server/config.wan.json5`, `relay-server/config.wan.e2e.json5`

---

## 横断評価層

### テスト戦略

- **WAN 実 host 一往復 E2E（host Accept / Frame・no-cert・guest sim put 非配送・旧 epoch・ワイルドカード不可読）（継続）** `+5`
  > 対象ファイル: `.scripts/smoke-compose-wan-e2e.sh`, `.github/workflows/e2e-compose.yml`

- **cargo-fuzz・ヘルメティック Compose E2E・auth の厚いテスト（当日 134 件）（継続）** `+9`
  > 対象ファイル: `.github/workflows/host-fuzz-ci.yml`, `docker-compose.e2e.yml`

- **Rust ユニットテストの厚み（当日 client workspace 708・protocol 124・relay-agent 32 件 PASS）（継続・増加）** `+5`
  > 前回 447 / 73 / 32 から大きく増えた。shared-sim の MVCC・SIMD・接触、host の edit レーン・sim_state、client の予測・jitter・cell 合流にそれぞれテストがある。+4 → +5。
  > 対象ファイル: `client/`, `protocol/`

- **relay-agent のテスト（継続）** `+2`
  > 対象ファイル: `relay-agent/src/lib.rs`

- **adversarial E2E・汚染パス隔離テスト・host Token 否定系・Rust / Elixir 共有ベクタ・JWKS 時計注入（継続）** `+8`
  > 対象ファイル: `client/e2e-probe/`, `protocol/testdata/`

- **assets テストで reaper / GC を起動しない hermetic 化（継続）** `+1`
  > 対象ファイル: `assets-server/config/test.exs`

- **world のテスト 162 → 232 件、権限の否定系（scene_writer・eligible・SpawnTicket・rotate）（継続・格上げ）** `+2`
  > 前回「takeover → scene 書き込みの否定系テストが無い」ことが -3 を見逃した原因だった。今回は `scene_writer_forbidden` を 4 箇所、非 owner の eligible / rotate 拒否、ticket 無しの 403 をそれぞれ入れている。+1 → +2。
  > 対象ファイル: `world-server/test/world_web/controllers/room_controller_test.exs`, `world-server/test/world/scene_writer_test.exs`

- **assets のテスト 191 → 240 件（VCS push・SpawnTicket・Origin Cert v2）（新規）** `+1`
  > 対象ファイル: `assets-server/test/`

- **shared-sim の `aarch64-linux-android` クロスチェックを CI に（新規）** `+1`
  > `rust-ci.yml` の `shared-sim-android-check`（NDK r26d）。Quest を本命とする方針を CI で守る。
  > 対象ファイル: `.github/workflows/rust-ci.yml`

### 可観測性

- **6 プロセスの loopback Prometheus・Room Token `jti` 相関・auth 構造化ログ・host 周期カウンタ（継続）** `+9`
  > 対象ファイル: `world-server/lib/world/metrics.ex`, `client/host-server/src/metrics.rs`

- **auth / world / assets の 3 サービスで `/ready` を liveness と分離（継続）** `+1`
  > 対象ファイル: `world-server/lib/world/ready.ex`, `auth-server/lib/auth/ready.ex`

### エラーハンドリング

- **anyhow + context / タグ付きタプル・ストレージ障害と不存在の分離・応答から `inspect` 除去・relay の lock `map_err`（継続）** `+6`
  > 対象ファイル: `world-server/lib/world_web/controllers/room_controller.ex`

- **CAS 系の失敗を 409 / 503 / 404 に意味で分ける（継続）** `+1`
  > overlay クリア失敗も、上限到達（`:invalid_revision`）と競合（`:conflict`）を区別する（`rooms/internal.ex` L91-98）。
  > 対象ファイル: `world-server/lib/world/rooms/internal.ex`

- **エラー → HTTP 変換を `FallbackController` の表 1 箇所に（新規・x00d6）** `+1`
  > 対象ファイル: `world-server/lib/world_web/controllers/fallback_controller.ex`

- **edit レーンの reject 理由コードと、失敗時に tip を巻き戻さない / ミラーを戻す回復（新規・x00b9）** `+1`
  > `edit_reject::{PAYLOAD, APPLY}` を Commit に載せ、inbox が閉じていれば `release_reserved` / `rollback_mirrors` してから reject を出す（`edit.rs` L116-200）。
  > 対象ファイル: `client/host-server/src/edit.rs`

### 変更容易性・保守性

- **モジュールごとの Guarantees / Non-guarantees・`shared/alchemy_*` 共有・`alchemy_auth_client`・shared-sim の単一正本・protocol ゲート・`undocumented_unsafe_blocks = "deny"`（継続）** `+13`
  > 新しい既定経路（`bus/frame.rs` L37-43 の cell 本線、`sim.rs` 冒頭の zero-alloc 非保証、`edit.rs` L15-18 の `control_seq` 窓）も、弱点を含めて冒頭に書かれている。今回の weaknesses の多くは、この「非保証」欄を読むだけで見つかった。
  > 対象ファイル: `client/Cargo.toml`, `shared/`

- **依存追加に人の承認日とライセンスをコメントで残す運用（新規）** `+1`
  > `wide`（`client/Cargo.toml` L43「Human-approved (2026-11-14)」）、`keyring`（`client/app/Cargo.toml` L41）、`ureq`（`relay-agent/Cargo.toml` L23）。guardrails の「新規依存は人に回す」を、コード上で追跡できる形にしている。
  > 対象ファイル: `client/Cargo.toml`, `client/app/Cargo.toml`, `relay-agent/Cargo.toml`

### 開発者体験（DX）

- **README → compose → cargo run、`ci.sh` 単一入口・dev / release 2 ステージ Docker・prod compose config・rust-cache（継続）** `+7`
  > 対象ファイル: `.scripts/ci.sh`, `README.md`

- **`ci.sh all` が relay-agent・shared 3 パッケージも回す（継続）** `+2`
  > 対象ファイル: `.scripts/ci.sh`

- **Windows ネイティブ（Git Bash + Docker Desktop）で完走（継続・当日 assets のみ初回失敗）** `+1`
  > 対象ファイル: `.scripts/ci.sh`

### セキュリティ・配布可能性

- **Untrusted host 前提の信頼チェーン（seal + attestation + gen/rev + discard + 部屋 AEAD + 短命 mTLS + 制御面署名 + HTTP PoP + 鍵 epoch）（継続）** `+5`
  > 今回、権限の付け方（HostDelegation・SpawnTicket・eligible）と資格情報の運び方（AUTH_PUSH・OS vault・PoP nonce）が加わり、「誰に何を渡すか」の層まで揃った。
  > 対象ファイル: `.workspace/0_doc/architecture/trust-boundary.md`

- **依存脆弱性管理（cargo deny ×2 lock・hex.audit）・入口防御の順序・release から既定資格情報除去・`host_control` 既定 OFF・`PUBLISH_ADDR` 既定 loopback（継続）** `+9`
  > 当日 cargo deny（client / relay-agent）PASS、hex.audit PASS。
  > 対象ファイル: `.github/workflows/rust-ci.yml`

- **Windows x64 テスター ZIP の自動化（継続）** `+2`
  > 対象ファイル: `.github/workflows/windows-release.yml`

- **HostSupervisor ↔ host を loopback IPC + セッショントークンに限定（継続）** `+1`
  > 対象ファイル: `protocol/src/host_ipc.rs`

- **`forbid(unsafe_code)` をクレート根で CI 強制（新規）** `+1`
  > `rust-ci.yml` の `forbid-unsafe-roots` が protocol・shared-sim・host-server・trust・zenoh-connect・vcs-store の根を grep し、`client/*/src/lib.rs` の新顔も既定で検査対象にする（OpenXR を使う app だけ明示除外）。Untrusted 入力を扱うクレートの安全性を、規約ではなくゲートで守る。
  > 対象ファイル: `.github/workflows/rust-ci.yml`

### プロジェクト全体設計

- **評価 → 改善計画 → スライス → PR の自己改善サイクルが、前回 P0 2 件・P1 6 件・P2 8 件を 1 週間でコードに落とした（継続・格上げ）** `+5`
  > x009d〜x00d6 の 58 スライス。前回の最大欠陥（takeover による canonical 改ざん）は権限モデルごと作り直して閉じ、GUI host の資格情報切れ・接触喪失・handover 同意・平文 refresh・GLB・KeyStale・relay-agent・一枚岩もコードで閉じた（判定は評価レポートの回帰表）。閉じた項目にはほぼすべて否定系テストが付いている。+4 → +5。
  > 対象ファイル: `.workspace/roadmap/slices/`

- **trust-boundary.md の継続保守（cell / lod・HostDelegation・PoP nonce・AUTH_PUSH を追記）（継続）** `+4`
  > edit / sim_state / SpawnTicket / VCS の行が無い点は weaknesses 参照。
  > 対象ファイル: `.workspace/0_doc/architecture/trust-boundary.md`

- **architecture/*.md の「保証する／しない」表とコードの一致（継続・Rapier / launcher 表記は解消）** `+3`
  > 対象ファイル: `.workspace/0_doc/architecture/`

- **4 階層マルチライセンスと注入境界・inventory ladder のスライス化・旧 Elixir 権威からの転換（継続）** `+6`
  > 対象ファイル: `LICENSE`, `TRADEMARKS.md`

- **改善計画にクローズ状況を追記する運用・パスフィルタ CI（継続）** `+2`
  > 対象ファイル: `.github/workflows/`

- **launcher を削除し責務を HostSupervisor に移した（継続）** `+1`
  > 対象ファイル: `.workspace/0_doc/architecture/launcher.md`

- **Spatial Interest を x00c7 で意図的に止め、止めた理由と再開条件を文書化（新規）** `+1`
  > `roadmap/memo/spatial-interest-pause-after-x00c7.md`。規模本線の機能を「どこまでで一旦止めるか」を先に決めてから進める運用は、個人プロジェクトでは珍しい。
  > 対象ファイル: `.workspace/roadmap/spatial_interest.md`, `.workspace/roadmap/memo/spatial-interest-pause-after-x00c7.md`
