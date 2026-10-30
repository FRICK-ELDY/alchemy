# Alchemy — プラス点詳細（第1評価者 Claude Opus 5.5 / 2026-10-30）

> 評価対象コミット: `d2d9f44`（main・PR #105 マージ直後）。前回 `6a0a879` から 90 コミット。
> 全項目は当日コードを直接読んで確認した。第2評価者（`gpt/`）の文書は参照していない。
> 「継続」は前回のプラス点で、当日に対象ファイル（または前回以降の差分が無いこと）と CI 通過を確認したもの。「新規」は前回以降に入った実装。

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
| 運営側コア | auth-server | +23 |
| 運営側コア | assets-server | +20 |
| 運営側コア | world-server | +23 |
| 共通契約 | protocol/ | +29 |
| ホスト端末 | client/host-server | +39 |
| ホスト端末 | client/app | +34 |
| ホスト端末 | launcher/ | +4 |
| 転送・エッジ | zenoh-router / edge / relay-agent | +15 |
| 横断 | テスト戦略 | +19 |
| 横断 | 可観測性 | +6 |
| 横断 | エラーハンドリング | +5 |
| 横断 | 変更容易性・保守性 | +11 |
| 横断 | 開発者体験（DX） | +9 |
| 横断 | セキュリティ・配布可能性 | +11 |
| 横断 | プロジェクト全体設計 | +14 |
| | **合計** | **+262** |

---

## 技術評価層 — 運営側コア

### auth-server（アイデンティティ SSoT）

> 前回以降の `auth-server/lib` の差分は `client_ip.ex`（共有化）・`plugs/rate_limit.ex`・`router.ex` のみ。当日 Docker 上で 122 件（format / compile `--warnings-as-errors` / credo `--strict` / hex.audit 込み）が通過した。

- **Refresh Token のファミリー回転・再利用検知・行ロック付き回転（継続）** `+4`
  > `FOR UPDATE` で回転を直列化し、失効済みトークン再提示はロールバック後にファミリー全失効。OAuth 2.0 Security BCP の rotation + reuse detection を実務水準で満たす。
  > 対象ファイル: `auth-server/lib/auth/accounts.ex`, `auth-server/test/auth/accounts_refresh_concurrency_test.exs`

- **RS256 + `kid` + ローテーション用検証鍵（継続）** `+3`
  > 署名鍵と検証専用鍵を分けて JWKS に全鍵を載せ、kid 重複は起動時に例外。prod は鍵の自動生成を禁止。
  > 対象ファイル: `auth-server/lib/auth/token/keys.ex`（L54-113）

- **多軸レート制限（IP × 識別子 × トークンファミリー）と初回超過のみ telemetry（継続）** `+3`
  > ETS `update_counter` の固定窓。古い窓は `select_delete` で定期掃除。
  > 対象ファイル: `auth-server/lib/auth/rate_limit.ex`（L73-88, L129-148）

- **Social mutate を `:authenticated` の後段で user + IP 両軸の rate limit に載せた（新規・x0043）** `+1`
  > 前回 -1（Follow / Friendship 無制限）を解消。認証後に置くことで user 軸のキーが取れる順序になっている。
  > 対象ファイル: `auth-server/lib/auth_web/router.ex`（L42-44）, `auth-server/lib/auth_web/plugs/rate_limit.ex`

- **Tunnel 背後のクライアント IP 解決（継続・共有化）** `+2`
  > 信頼プロキシからのみ `CF-Connecting-IP` を採用。x004b で `AlchemyClientIp` に共通化された（保守性の項も参照）。
  > 対象ファイル: `auth-server/lib/auth_web/client_ip.ex`, `shared/alchemy_client_ip/lib/alchemy_client_ip.ex`

- **Follow / Friendship の DB 制約付きソーシャルグラフ（継続）** `+2`
  > 正規化ペアの一意性と CHECK 制約で不変条件を DB に持たせる。
  > 対象ファイル: `auth-server/lib/auth/social.ex`, `auth-server/priv/repo/migrations/20261028*`

- **アカウント列挙・タイミング攻撃への配慮（継続）** `+2`
  > 対象ファイル: `auth-server/lib/auth/accounts.ex`

- **メール確認・パスワードリセットのトランザクション設計（継続）** `+2`
  > 対象ファイル: `auth-server/lib/auth/accounts.ex`

- **認証 Plug の失敗分類と情報を漏らさない構造化ログ（継続）** `+2`
  > 対象ファイル: `auth-server/lib/auth_web/plugs/authenticate.ex`

- **S2S 内部トークンの prod fail-closed と定数時間比較（継続）** `+1`
  > 対象ファイル: `auth-server/config/runtime.exs`

- **公開 / レート制限 / 認証必須 / 内部のパイプライン分離（継続）** `+1`
  > 対象ファイル: `auth-server/lib/auth_web/router.ex`（L8-61）

### assets-server（CAS・Quota・門番）

- **`CasObject`（物理・所有者なし）/ `BlobRef`（owner×blake3）/ `BlobGrant`（権利）の 3 分離（新規・x003f / x004f）** `+3`
  > 前回 -2 の「CAS object と owner reference の混同」を正面から解いた。別 owner が同じバイトを上げると自分の ref・自分の Grant・自分の Quota 消費が作られ、物理バイトは `put_if_absent` で 1 つだけ。`access` / mime は先勝ちで、異なる `access` を要求すると 409 `access_conflict` を返して「黙って先勝ち成功」にしない。git LFS / IPFS の pin と同じ「物理と参照の分離」を、権利台帳まで含めて小さく正しく実装している。
  > 対象ファイル: `assets-server/lib/assets/blobs.ex`（L36-60, L195-236）, `.workspace/0_doc/architecture/assets-server.md`（L54-81）

- **Quota 予約 → TX 外 CAS put → 確定、補償は予約解放のみで CAS を消さない（新規・x003f）** `+3`
  > 前回 -1（R2 アップロード中に SQLite 書き込みロックを保持）を解消。予約は短い TX、R2 への put は TX 外、確定で `reserved → used` 振替。補償で `CAS.delete` しない理由（同一 hash の並行 PUT と競合する）がコードと保証表の両方に書かれている。予約時の `used + reserved + add <= limit` 条件付き UPDATE で原子性も保つ。テスト専用の finalize 失敗フックは `Mix.env() == :test` で release から到達不能。
  > 対象ファイル: `assets-server/lib/assets/blobs.ex`（L1-12, L151-221, L465-516）

- **GET の `access` を fail-closed に扱い、存在オラクルを文書で明示（新規・x004f）** `+2`
  > 既知の `authenticated` / `grant_required` だけを許可し、未知値は 403（公開に落とさない）。404 と 403 で存在が区別できることを「意図」として保証表に書いている。期待される認可拒否はエラーメトリクスに数えない、という細部も良い。
  > 対象ファイル: `assets-server/lib/assets/blobs.ex`（L348-404）, `.workspace/0_doc/architecture/assets-server.md`（L36-50）

- **`access_conflict` を 409 で明示し、メトリクス上もインフラ失敗と区別（新規）** `+1`
  > 対象ファイル: `assets-server/lib/assets/blobs.ex`（L62-71）, `assets-server/lib/assets_web/controllers/blob_controller.ex`（L99-103）

- **メタと不変バイナリの分離 + BLAKE3 CAS（継続）** `+2`
  > 対象ファイル: `assets-server/lib/assets/cas.ex`

- **CAS バックエンドの behaviour 抽象と起動時の必須 env 検査（継続）** `+2`
  > 対象ファイル: `assets-server/lib/assets/cas/backend.ex`, `assets-server/lib/assets/cas/s3.ex`

- **s3 時の GET を presigned URL への 302 にする（継続）** `+2`
  > 対象ファイル: `assets-server/lib/assets/blobs.ex`（L420-425）

- **条件付き UPDATE による Quota の原子更新（継続）** `+2`
  > 対象ファイル: `assets-server/lib/assets/blobs.ex`（L464-479）

- **`Plug.Parsers` 撤去による生 body 受信（継続）** `+1`
  > 対象ファイル: `assets-server/lib/assets_web/controllers/blob_controller.ex`（L8-35）

- **テスト環境で CAS 系 env を無視する隔離（継続）** `+1`
  > 対象ファイル: `assets-server/config/runtime.exs`, `.scripts/ci.sh`（L146-150）

- **local CAS の原子的書き込み（tmp → rename）（継続）** `+1`
  > 対象ファイル: `assets-server/lib/assets/cas/local.ex`

### world-server（案内所・門番・監査）

- **ManifestSeal を DB の scene + 検証済み attestation から world 自身が計算する（新規・x0039）** `+4`
  > 前回まとめ -4 の「seal が任意 body hash の署名 oracle」を閉じた。`CanonicalManifest.canonical_blake3/2` は attestation を検証し、`Rooms.match_claimed_host` で現 host 鍵と世代を照合し、DB の scene から protobuf を組み立てて BLAKE3 を出す。host が送る `revision` / `manifest_blake3` は「一致確認」にだけ使い、不一致は 422。エンコードは Rust の `alchemy-protocol` を CLI 経由で共有するので、Elixir と Rust で protobuf の正規化が二重実装にならない。Matrix の server signing と同じ「中央が自分のデータから署名対象を作る」形で、署名の意味が「host が言ったもの」から「world が定義したもの」に変わった。
  > 対象ファイル: `world-server/lib/world/canonical_manifest.ex`（L13-32）, `world-server/lib/world_web/controllers/room_controller.ex`（L352-397, L506-510）, `protocol/src/bin/manifest_seal_hash.rs`

- **Room Token に `cnf.spk`（RFC 7800 相当）を埋め込む（新規・x003e）** `+2`
  > 32 バイトの `session_pub` を base64url で claim に入れ、verify 側でも `valid_cnf?` を必須 claim にした。host 側の照合は host-server の項。
  > 対象ファイル: `world-server/lib/world/token.ex`（L13-40, L66-84）

- **`wire_cap` の発行と Allocation Token（`aud=alchemy-relay`・`relay_id` 束縛）の分離（新規・x004e）** `+1`
  > Room Token と aud を分け、relay 側で `relay_id` 一致を要求する設計は正しい（`wire_cap` の秘匿性の問題は弱点参照）。
  > 対象ファイル: `world-server/lib/world/allocation_token.ex`, `world-server/lib/world/rooms.ex`（L347-356）

- **HostAttestation / ManifestSeal の発行（typ・aud 分離の RS256 JWT）と host_generation（継続・拡張）** `+3`
  > 対象ファイル: `world-server/lib/world/host_attestation.ex`, `world-server/lib/world/manifest_seal.ex`

- **Room の永続化（Ash + SQLite）とストレージ障害の 503 分離（継続）** `+2`
  > 対象ファイル: `world-server/lib/world/rooms.ex`, `world-server/lib/world_web/controllers/room_controller.ex`（L532-536）

- **`host` ロールは owner のみ（継続）** `+2`
  > 対象ファイル: `world-server/lib/world_web/controllers/room_controller.ex`（L359-363, L522-530）

- **公開範囲の Token 発行時 ACL（auth への S2S・fail-closed）（継続）** `+2`
  > 対象ファイル: `world-server/lib/world/auth_access.ex`

- **Room Token の claim 設計（継続）** `+2`
  > 対象ファイル: `world-server/lib/world/token.ex`

- **auth JWT 検証と `sub` の転写（継続）** `+2`
  > 対象ファイル: `world-server/lib/world_web/plugs/`

- **`lan` 公開範囲の fail-closed 判定（Tunnel ヘッダがあれば LAN 扱いしない）（継続）** `+1`
  > 対象ファイル: `world-server/lib/world_web/client_ip.ex`（L13-35）

- **匿名トークン既定 OFF（継続）** `+1`
  > 対象ファイル: `world-server/config/runtime.exs`（L19-26）

- **room_id 検証を protocol と共有ベクタで一致（継続）** `+1`
  > 対象ファイル: `world-server/lib/world/room_id.ex`

---

## 技術評価層 — 共通契約層

### protocol/（ワイヤ契約 SSoT）

- **decode 前のバイト上限 + decode 後の件数・文字列上限をクラス別に定義（新規・x003b）** `+3`
  > 前回まとめ -3 の「peer の protobuf を上限なしで decode」を閉じた。Manifest 1MiB / Frame 256KiB / Presence 64KiB / Input 32KiB の pre-decode 上限と、entries・upserts・JWT 文字列・`session_pub` 長の post-decode 上限を 1 ファイルに集約。「これは DoS 境界であって host の定常配信サイズではない」「上げるなら host 側ガードと同時に」と運用上の落とし穴まで moduledoc に書いている。client は `ZBytes::len()` で連続コピー前に弾く。
  > 対象ファイル: `protocol/src/decode_limits.rs`（L1-63）, `client/app/src/main.rs`（L345-360）

- **`(host_generation, revision)` 単調ゲートを protocol の純関数として host / client で共有（新規・x003a）** `+3`
  > 前回まとめ -4 の replay を閉じた。`decide_gen_rev` が `RejectRegression` / `RejectInvalidGeneration` / `AcceptSameOrForward` を返し、同一世代内では `host_pub` を固定する。client は `host_pub` と `gen_rev` の両ロックを保持したまま判定と適用を行い、片方だけ更新される状態を作らない。
  > 対象ファイル: `protocol/src/generation_gate.rs`, `client/app/src/main.rs`（L403-430）

- **`manifest-seal-hash` バイナリで Elixir に Rust のエンコード経路を提供（新規・x0039）** `+2`
  > NIF を避けつつ、canonical Manifest の protobuf エンコードの正本を Rust 1 箇所に保つ。
  > 対象ファイル: `protocol/src/bin/manifest_seal_hash.rs`

- **`user_root_slots_from_entries` で部屋容量を Manifest から導出（新規・x004d）** `+1`
  > 重複・不連続を型付きエラーで拒否する。天井の問題は弱点参照。
  > 対象ファイル: `protocol/src/lib.rs`（L439-470, L830-877）

- **チャネル責務の分離（Manifest / frame / presence / input / liveliness / snapshot）（継続）** `+3`
  > 対象ファイル: `protocol/proto/*.proto`, `protocol/src/lib.rs`（L323-354）

- **Ed25519 ワイヤ署名（Intent・Frame・Presence）の `wire_crypto` 集約（継続）** `+3`
  > 対象ファイル: `protocol/src/wire_crypto.rs`

- **配送クラス契約（peer-delivery.md）と `decide_frame_tick`（継続）** `+3`
  > 対象ファイル: `protocol/peer-delivery.md`, `protocol/src/lib.rs`

- **`PROTOCOL_PIN` をワイヤに載せ不一致を検出（継続）** `+2`
  > 対象ファイル: `protocol/src/lib.rs`

- **TRS と正規化ヘルパ（継続）** `+2`
  > 対象ファイル: `protocol/src/lib.rs`

- **role → capability の写像（`sim_capability`）（継続）** `+2`
  > 対象ファイル: `protocol/src/sim_capability.rs`

- **client 外の独立クレート + protoc スキーマ検査（継続）** `+2`
  > 当日 `protoc` の descriptor コンパイルも PASS。
  > 対象ファイル: `protocol/Cargo.toml`, `.scripts/ci.sh`（L53-64）

- **キー式ヘルパと liveliness キーの `(sub, jti)` 解析、`validate_wire_cap`（継続・拡張）** `+2`
  > 対象ファイル: `protocol/src/lib.rs`（L276-325, L403-435）

- **`validate_room_id` を world と同じ規則に統一（継続）** `+1`
  > 対象ファイル: `protocol/src/lib.rs`

---

## 技術評価層 — ホストユーザー端末

### client/host-server（Untrusted 計算ホスト）

- **credential worker と「fail-closed = 失効で配信停止、失敗で殺さない」（新規・x003d）** `+4`
  > 前回双方 -2 の「物理ループ内で同期 HTTP、1 回失敗でプロセス終了」を、設計の言葉ごと正した。資格情報と seal の HTTP は専用 worker に移り、物理ループは毎 tick `read_publish_gate` で gate と `hard_expire_at` を読むだけ。soft 期限での失敗はバックオフ再試行で直前の正常 snapshot を保持し、hard 期限を過ぎたら配信だけを止めて物理は続ける。再開時は `last_sent` から remove を合成して catch-up する。moduledoc の「Fail-closed means stop publishing after hard expiry, not kill the process on a transient 502」は前回の講評をそのまま設計原則に昇格させている。
  > 対象ファイル: `client/host-server/src/room_token.rs`（L10-13）, `client/host-server/src/main.rs`（L983-1017, L1149-1153）

- **Zenoh put を network worker に分離し、物理はジョブ投入のみ（新規・x0042）** `+3`
  > Frame / Manifest / Presence の encode・署名・put は `zenoh-publish` worker で行い、物理ループは `submit_*` するだけ。worker 側で coalesce し、件数をメトリクスに出す。Zenoh の一時失敗で物理が止まらない。
  > 対象ファイル: `client/host-server/src/network_worker.rs`, `client/host-server/src/main.rs`（L937-946）

- **bind で `cnf.spk == session_pub` を照合（新規・x003e）** `+3`
  > 前回 -2（bind 時 JWT の盗聴で別鍵 rebind）を閉じた。盗聴した JWT だけでは別の鍵で bind できない。否定系テストもある。
  > 対象ファイル: `client/host-server/src/session.rs`（L1-30, L115-122, L500-515）

- **Intent rate limit の「どこで数えるか」の設計（新規・x0040）** `+3`
  > steady は Ed25519 成功後・seq 確定前に消費（偽署名で他人のバケットを減らせない）、bind は Ed25519 後・JWT 前に host 全体と per-pub の両方を「両方あるときだけ消費」、失敗 bind は未 bind の per-pub エントリを忘れる、Presence 回収と連動して GC。順序の各段に理由が書かれている（全体バケットの枯渇は弱点参照）。
  > 対象ファイル: `client/host-server/src/intent_rate.rs`（L1-30）, `client/host-server/src/main.rs`（L1036-1046）

- **JWKS の hard stale（300s）で撤去鍵の無限受理を止める（新規・x0040）** `+1`
  > 対象ファイル: `client/host-server/src/room_token.rs`（L36-41, L221-250, L312-321）

- **Grab 保持中の aim / offset を開始距離と同じ定数でクランプ（新規・x0041）** `+1`
  > 前回 -1 を解消。「clamp を外すと 2·MAX を超える」ことを回帰テストで固定。
  > 対象ファイル: `client/host-server/src/sim.rs`（L331, L352, L951-1000）

- **Intent セッション表（bind / steady / 単調 seq / 1 sub 1 セッション）（継続）** `+3`
  > 対象ファイル: `client/host-server/src/session.rs`

- **world 定義の Scene から Rapier を構築（Manifest 駆動）（継続）** `+3`
  > 対象ファイル: `client/host-server/src/main.rs`（L785-800, L948）, `client/host-server/src/sim.rs`

- **Presence 解放の三重化と、liveliness を検証済み jti に紐付け（継続）** `+3`
  > 対象ファイル: `client/host-server/src/main.rs`（L699-733, L1021-1047）

- **JWKS の SWR とバックグラウンド更新、未知 kid のクールダウン（継続）** `+2`
  > 対象ファイル: `client/host-server/src/room_token.rs`

- **move の単位球クランプと Grab 開始距離の判定（継続）** `+2`
  > 対象ファイル: `client/host-server/src/finite.rs`

- **snapshot queryable による純デルタ化と tick 規則（継続）** `+2`
  > hard expiry 中は snapshot も `reply_err` で返さない、という一貫性が加わった。
  > 対象ファイル: `client/host-server/src/main.rs`（L652-697）

- **内部物理 60Hz と配信 30Hz の分離 + 固定ステップ + tick lateness 計測（継続・拡張）** `+2`
  > 対象ファイル: `client/host-server/src/main.rs`（L959-981, L1097-1117）

- **入力ゲートの順序と拒否理由ごとのカウンタ（継続）** `+2`
  > 対象ファイル: `client/host-server/src/main.rs`（L459-650）

- **Room Token 検証の厳格さ（alg 固定・iss/aud/exp・room_id 束縛）（継続）** `+2`
  > 対象ファイル: `client/host-server/src/room_token.rs`

- **Rapier の SoA 構成と Grab の no-steal（継続）** `+2`
  > 対象ファイル: `client/host-server/src/sim.rs`

- **Mutex の poison を致命的エラーとして扱う（継続）** `+1`
  > 対象ファイル: `client/host-server/src/main.rs`（L48-52, L421-456）

### client/app（体感・描画・UI エンドポイント）

- **Manifest / Presence の検証失敗は discard + 理由別カウンタ、退室は明示 Leave のみ（新規・x0038）** `+4`
  > 前回 -3（偽 Manifest 1 パケットで全員退室）を閉じた。pin / room_id / host_pub 長 / seal / attestation / gen-rev / decode のいずれの失敗も `break 'sample` で破棄し、`dropped_identity` 等の 6 種のカウンタを周期ログに集約する。`leave_room_local()` を呼ぶのは明示 Leave の経路（L1621）だけになった。「署名は偽物を受け入れないためのもので、偽物を見たら逃げるためのものではない」という前回の指摘を、ゲート関数の doc（"Failure → discard sample (never leave)"）とテスト（L1919）で固定している。
  > 対象ファイル: `client/app/src/main.rs`（L102-127, L336-430, L1595-1622, L1919）

- **gen/rev ゲートを `host_pub` と `gen_rev` の両ロック保持下で判定・適用（新規・x003a）** `+3`
  > poison や同一世代の鍵すり替えで `host_pub` だけが更新される中間状態を作らない。並行性の細部まで考えられている。
  > 対象ファイル: `client/app/src/main.rs`（L403-430）

- **XR 描画を wgpu SceneGpu に統合し、2720 行の生 Vulkan 経路を削除（新規・x0050〜x0052）** `+4`
  > 前回 -2（描画経路二重化）を閉じた。XR の Vulkan を wgpu-hal で採用して desktop と同じ `SceneGpu`（`scene.rs`）を dual-eye で描き、UI / レーザー / コントローラは overlays に分離。ash は enable / interop のみに縮小し、`vk_loop.rs`（2720 行）を削除した。3 スライス（clear-only → SceneGpu → 旧経路削除）に分けて段階移行した手順も良い。Bevy の `bevy_openxr` と同じ「1 つのレンダラを複数 surface で使う」形に到達した。
  > 対象ファイル: `client/app/src/surface/xr/mod.rs`（L11, L66-69）, `client/app/src/surface/xr/wgpu_session.rs`, `client/app/src/surface/xr/wgpu_interop.rs`, `client/app/src/scene.rs`（L671）

- **assets 失敗は placeholder、release では Coin fixture に落ちない、HTTP に connect / read timeout（新規・x0048）** `+2`
  > 前回まとめ -2 を閉じた。fixture は `cfg!(debug_assertions)` かつ env 指定時のみ。
  > 対象ファイル: `client/app/src/main.rs`（L95-100, L148-158, L196-265）

- **client world JWKS の soft / hard + 未知 kid クールダウン（受信パスで HTTP なし）（新規・x0044）** `+2`
  > 前回 -1 を解消。host と同型でテスト付き。
  > 対象ファイル: `client/app/src/world_jwks.rs`（L1-25, L155-250, L329-420）

- **Frame / Presence の署名検証と、seal + attestation の信頼チェーン（継続）** `+3`
  > 対象ファイル: `client/app/src/main.rs`, `client/app/src/manifest_seal.rs`, `client/app/src/host_attestation.rs`

- **system_ui が OpenXR / winit / Room を知らない（継続）** `+3`
  > 対象ファイル: `client/app/src/system_ui/`

- **OpenXR の実機コントローラ描画（render model・animatable node）（継続）** `+3`
  > wgpu 統合後も維持された。
  > 対象ファイル: `client/app/src/surface/xr/render_model.rs`

- **access / Room Token の 80% 自動更新と liveliness の付け替え（継続）** `+2`
  > 対象ファイル: `client/app/src/main.rs`

- **surface の自動選択とフォールバック（継続）** `+2`
  > 対象ファイル: `client/app/src/surface/mod.rs`, `client/app/src/surface/xr/mod.rs`（L33-62）

- **デコード前の BLAKE3 照合と 302 追従（継続）** `+2`
  > 対象ファイル: `client/app/src/main.rs`（L250-264）

- **ログイン・入室の HTTP をワーカースレッドで実行（継続）** `+1`
- **新規ワールド作成とアクセスレベル選択（仮 UI）（継続）** `+1`
- **Leave → liveliness undeclare → session close の順序設計（継続）** `+1`
  > 対象ファイル: `client/app/src/main.rs`（L1613-1621）
- **サブルーター化しない純粋エンドポイント（継続）** `+1`

### launcher/（プロセスマネージャ）

- **再起動ポリシーを純関数 `decide_host_exit` に切り出し、テストと CI に載せた（新規・x0047）** `+2`
  > 前回 -1（CI 対象外・clippy 失敗・テスト 0）を解消。当日 fmt / clippy / build / test（4 件）が PASS。
  > 対象ファイル: `launcher/src/restart.rs`, `.github/workflows/rust-ci.yml`（L101-121）

- **ready ファイルでの IPC アドレス通知（TOCTOU なし）と終了理由の区別（継続）** `+2`
  > 対象ファイル: `launcher/src/main.rs`（L1-11, L53-60）

---

## 技術評価層 — 転送・公開エッジ

### zenoh-router / edge / relay-agent

- **WAN を TLS + mTLS + `default_permission: deny` + role CN ACL にした（新規・x004e）** `+3`
  > 前回まとめ -2（TLS のみ・publisher 隔離なし）に対し、mTLS 未提示 peer の接続拒否と、guest は manifest / frame / presence / snapshot に put 不可、host は input に put 不可という「キー族 × 役割」の静的 ACL を入れた。ACL ホットリロードが無い Zenoh の制約から role 固定 CN を選んだ判断理由も記録されている（部屋隔離の穴は弱点参照）。
  > 対象ファイル: `zenoh-router/config.wan.json5`（L21-119）

- **Relay Agent の Allocation 検証（RS256 固定・iss / aud / relay_id / role の全照合）（新規・x004e）** `+1`
  > 新しい Cargo の暗号クレートを入れず openssl CLI と既存 `jsonwebtoken` で済ませ、CA 署名は Mutex で直列化、serial は nanos で一意化する。ガードレール（依存追加は人に回す）を守った実装。
  > 対象ファイル: `relay-agent/src/main.rs`（L186-217, L219-230）

- **`/internal/*` を Tunnel ingress で 404（新規・x0043）** `+1`
  > 前回 -1 を解消。`/dev/*` と同型。
  > 対象ファイル: `edge/cloudflared/config.yml.example`（L17-23）

- **prod / release 前提の Tunnel 手順と `/dev/*` の ingress 遮断（継続）** `+2`
- **WAN の Zenoh を TLS 化し、接続ヘルパを `zenoh-connect` に共通化（継続・mTLS 拡張）** `+2`
  > 対象ファイル: `client/zenoh-connect/`
- **ローカル / WAN 設定の分離と差分の明記（継続）** `+2`
  > 対象ファイル: `zenoh-router/config.json5`, `zenoh-router/config.wan.json5`, `.workspace/0_doc/architecture/zenoh-router.md`（L17-22）
- **WAN の運用手順（systemd・ufw allowlist・relay 起動）（継続）** `+1`
  > 対象ファイル: `.workspace/0_doc/manual/zenoh-wan-conoha.md`
- **「zenohd はトークンを見ない」という境界の明記（継続）** `+1`
- **`TRUSTED_PROXIES` の必須手順化（継続）** `+2`
  > 対象ファイル: `edge/cloudflared/README.md`

---

## 横断評価層

### テスト戦略

> 当日の件数: Rust（client 105 / host lib 47 + bin 53 / shared-sim 3 / launcher 4 / protocol 39）、auth 122、world 65、assets 34。前回 Rust 186 → 251、world 49 → 65、assets 13 → 34。

- **cargo-fuzz（protobuf decode + 上限、Room Token verify）を CI で短時間実行（新規・x004c）** `+3`
  > 前回提案を実装。protocol と host の両方に fuzz ターゲットを置き、`FUZZ_MAX_TOTAL_TIME` で CI 時間を抑えた smoke として GitHub Actions に入れた。ワイヤ境界の decoder を fuzz する個人プロジェクトは稀。
  > 対象ファイル: `protocol/fuzz/`, `client/host-server/fuzz/fuzz_targets/room_token_verify.rs`, `.github/workflows/protocol-ci.yml`（L57-76）, `.github/workflows/host-fuzz-ci.yml`

- **adversarial E2E（偽 Manifest・seal 無し stale-rev・oversize Input / Manifest）（新規・x0046）** `+2`
  > 1 ステップごとに独立した待ち時間を持ち、前段の遅延で後段の assert が飢えないようにしてある（範囲の不足は弱点参照）。
  > 対象ファイル: `client/e2e-probe/src/main.rs`（L190-247, L434-519）

- **テスト環境の汚染を「汚染パスを渡しても無視されること」で証明する隔離テスト（新規・x003b）** `+2`
  > 前回 -2（world テストが開発 DB を読む）を閉じた。runtime.exs は `config_env() != :test` のときだけ `WORLD_DATABASE_PATH` を適用し、ci.sh はわざと `WORLD_DATABASE_PATH=/tmp/alchemy-world-test-must-ignore.db` を渡して隔離を検証する。`test_env_isolation_test.exs` で固定。assets の教訓を横展開した。
  > 対象ファイル: `world-server/config/runtime.exs`（L58-63）, `.scripts/ci.sh`（L118-123）, `world-server/test/world/test_env_isolation_test.exs`

- **host Room Token の否定系マトリクス（alg=none / iss / aud / exp / room / role）（新規・x0045）** `+1`
  > RSA 鍵 1 本をマトリクス全体で共有して CI 時間を抑えている。
  > 対象ファイル: `client/host-server/src/room_token.rs`（L2012-）

- **ヘルメティックな Compose 縦 E2E を GitHub Actions で実行（継続）** `+3`
  > 対象ファイル: `.github/workflows/e2e-compose.yml`, `docker-compose.e2e.yml`
- **auth-server の厚いテスト（継続）** `+3`
- **Rust のユニットテスト（継続・増加）** `+3`
- **Rust と Elixir の共有テストベクタ（継続）** `+1`
- **共有 JWKS キャッシュのテスト（時計を注入）（継続）** `+1`
  > 対象ファイル: `shared/alchemy_auth_client/test/alchemy_auth_client/jwks_test.exs`

### 可観測性

- **host / client / assets に loopback 専用 Prometheus テキスト（低 cardinality・依存なし）（新規・x0049）** `+3`
  > tick lateness・late>5ms カウンタ・frame age・JWKS age / hard-stale・CAS put/get と所要時間。公開ポートに載せず専用 loopback bind にした理由（Tunnel → 127.0.0.1 で peer が loopback に見える経路を避ける）が trust-boundary.md に書かれている。`sub` / `room_id` をラベルにしない cardinality 方針も明示。新規依存を入れずに Prometheus 形式を手で出している。
  > 対象ファイル: `client/host-server/src/metrics.rs`, `client/app/src/metrics.rs`, `assets-server/lib/assets/metrics.ex`, `assets-server/lib/assets/metrics/server.ex`, `.workspace/0_doc/architecture/trust-boundary.md`（L70）

- **auth の構造化失敗ログと telemetry（継続）** `+2`
- **host の周期カウンタ（publish / upsert / coalesce / net_tick）（継続・拡張）** `+1`
  > 対象ファイル: `client/host-server/src/main.rs`（L1155-1183）

### エラーハンドリング

- **応答から `inspect(reason)` を除去し、分類コードのみ返す（新規・x004b）** `+1`
  > 前回 -1 を解消。world / assets の lib に `detail:` は残っていない（Phoenix 既定の `error_json.ex` を除く）。内部理由は Logger のみ。
  > 対象ファイル: `world-server/lib/world_web/controllers/room_controller.ex`（L383-497）, `assets-server/lib/assets_web/controllers/blob_controller.ex`（L105-111）

- **Rust は anyhow + context、Elixir はタグ付きタプル（継続）** `+2`
- **ストレージ障害と「存在しない」の分離（継続）** `+2`

### 変更容易性・保守性

- **`ClientIp` を `shared/alchemy_client_ip` に共有化し、各サービスは薄い委譲だけ（新規・x004b）** `+2`
  > 前回 -1 を解消。world 固有の `lan_eligible?` だけを world 側に残し、信頼プロキシ判定は 1 箇所になった。独立した CI workflow もある。
  > 対象ファイル: `shared/alchemy_client_ip/lib/alchemy_client_ip.ex`, `auth-server/lib/auth_web/client_ip.ex`（L1-8）, `world-server/lib/world_web/client_ip.ex`（L1-11）, `.github/workflows/alchemy-client-ip-ci.yml`

- **ゲート（gen/rev・decode 上限）を protocol に置き、host / client で同じ関数を使う（新規）** `+1`
  > 対象ファイル: `protocol/src/generation_gate.rs`, `protocol/src/decode_limits.rs`

- **保証単位のファイル分割と、モジュールごとの Guarantees / Non-guarantees（継続）** `+3`
  > 新規モジュール（`intent_rate.rs`・`network_worker.rs`・`decode_limits.rs`・`canonical_manifest.ex`・relay-agent）もすべてこの形式を守っている（`room_token.rs` の肥大は弱点参照）。
- **JWKS キャッシュと JWT 検証を `alchemy_auth_client` に共通化（継続）** `+2`
- **Scene 分類の単一正本（`alchemy-shared-sim`）（継続）** `+2`
- **`undocumented_unsafe_blocks = "deny"`（継続）** `+1`
  > 対象ファイル: `client/Cargo.toml`（L16-18）

### 開発者体験（DX）

- **`make ci` が Windows ネイティブ（Git Bash + Docker Desktop）で全緑、cargo deny / protoc も実行（新規）** `+2`
  > 当日 `ci.sh all` が `RESULT: ALL PASSED`。前回は world と launcher が赤だった。Windows 日常開発をネイティブ主軸に切り替えた文書（PR #100）とも整合。
  > 対象ファイル: `.scripts/ci.sh`, `Makefile`

- **prod compose の config 検証を CI に、`SMOKE_PROD_UP=1` で /health 起動まで（新規・x003b）** `+1`
  > 前回 -3（prod compose が起動しない）を閉じた上で、再発防止を CI に入れた。使い捨ての秘密を openssl で生成する。
  > 対象ファイル: `.scripts/smoke-prod-compose.sh`, `.github/workflows/prod-compose-smoke.yml`

- **README → compose → cargo run、`make ci` を単一入口に（継続）** `+3`
- **dev / release の 2 ステージ Docker と prod compose（継続）** `+2`
- **Rust CI に `Swatinem/rust-cache`（継続）** `+1`

### セキュリティ・配布可能性

- **Untrusted host 前提の信頼チェーンが「意味のある署名」まで閉じた（新規）** `+4`
  > canonical seal（world が定義）→ HostAttestation（鍵と世代）→ `(generation, revision)` 単調ゲート（replay 拒否）→ 同一世代内 `host_pub` 固定 → Frame / Presence の `host_sig` → 失敗は discard、入力側は `cnf` + Ed25519 session + rate + finite + クランプ。前回まとめの P0 暗号契約 4 項目（leave on fail / seal oracle / replay / decode limit）がすべてコードで閉じ、それぞれにテストがある。Resonite / VRChat 系で非公開のまま運営者を信頼させている部分を、個人プロジェクトが脅威表付きで公開実装している。
  > 対象ファイル: `world-server/lib/world/canonical_manifest.ex`, `protocol/src/generation_gate.rs`, `client/app/src/main.rs`（L336-430）, `client/host-server/src/session.rs`

- **Untrusted 入力の入口防御（署名 → JWT → caps → finite → クランプ）（継続）** `+2`
- **release ビルドから既定資格情報を除去（継続）** `+2`
  > 対象ファイル: `client/host-server/src/room_token.rs`（L1551-）
- **Zenoh `host_control` 既定 OFF、launcher はローカル IPC（継続）** `+2`
- **依存の脆弱性管理（cargo deny を CI でゲート、hex.audit）（継続）** `+1`
  > 当日ローカルでも `cargo deny check` が PASS（対象範囲の不足は弱点参照）。

### プロジェクト全体設計

- **architecture/*.md の「保証する／しない」表とコードの一致（継続・一部劣化）** `+3`
  > assets-server.md の GET capability 表（存在オラクルの意図まで）、client.md の host 行（credential worker / network worker / hard expiry）、trust-boundary.md の各行が当日コードと一致した。前回 +4 から、client.md のキー表と `wire_cap` 隔離の過大主張（弱点参照）で 1 点下げた。
  > 対象ファイル: `.workspace/0_doc/architecture/*.md`

- **信頼境界の脅威モデル（trust-boundary.md）の継続保守（継続）** `+3`
  > x0038〜x004e の各防御が表の行に反映され、参照スライス表で追跡できる。
  > 対象ファイル: `.workspace/0_doc/architecture/trust-boundary.md`

- **評価 → 改善計画 → スライス → PR の自己改善サイクル（継続・成果）** `+3`
  > 前回の改善計画 P0〜P3 の項目の大半にスライス ID と done が付き、当日のコード再読でも前回弱点 34 件中 20 件が closed、8 件が partial と確認できた（回帰表は総合レポート）。見送り（P3-7）も「CI が完了しなくなるため」と理由付きで記録している。
  > 対象ファイル: `.workspace/0_doc/evaluation/archive/2026-10-28/improvement-plan-2026-10-28.md`, `.workspace/roadmap/slices/`

- **インベントリラダー（x0054〜x0065）を細スライスに分け、保証表は「未実装のまま」と明記（新規）** `+1`
  > planned 文書を正本の保証表に混ぜず、「実装時に Slice 完了と同時更新」と書いて文書とコードの一致を守っている。
  > 対象ファイル: `.workspace/roadmap/inventory-ladder.md`, `.workspace/0_doc/architecture/assets-server.md`（L110）

- **改善計画のクローズ状況を計画書側に追記する運用（新規）** `+1`
  > 対象ファイル: `.workspace/0_doc/evaluation/archive/2026-10-28/improvement-plan-2026-10-28.md`（L22-80）

- **旧 Elixir 権威実行からの転換（継続）** `+2`
- **コンポーネント単位でパスフィルタした CI（継続）** `+1`
  > 対象ファイル: `.github/workflows/*.yml`
