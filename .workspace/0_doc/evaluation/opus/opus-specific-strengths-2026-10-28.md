# Alchemy — プラス点詳細（第1評価者 Claude Opus 5.5 / 2026-10-28）

> 評価対象コミット: `6a0a879`（main・PR #64 マージ直後）。前回 `683e30f` から 23 本のスライス（x001e〜x0037）がマージされている。
> 全項目は当日コードを直接読んで確認した。第2評価者の文書・まとめ文書は参照していない。

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
| 運営側コア | auth-server | +22 |
| 運営側コア | assets-server | +11 |
| 運営側コア | world-server | +16 |
| 共通契約 | protocol/ | +20 |
| ホスト端末 | client/host-server | +24 |
| ホスト端末 | client/app | +19 |
| ホスト端末 | launcher/ | +2 |
| 転送・エッジ | zenoh-router / edge | +10 |
| 横断 | テスト戦略 | +11 |
| 横断 | 可観測性 | +3 |
| 横断 | エラーハンドリング | +4 |
| 横断 | 変更容易性・保守性 | +8 |
| 横断 | 開発者体験（DX） | +6 |
| 横断 | セキュリティ・配布可能性 | +7 |
| 横断 | プロジェクト全体設計 | +13 |
| | **合計** | **+176** |

---

## 技術評価層 — 運営側コア

### auth-server（アイデンティティ SSoT）

- **Refresh Token のファミリー回転・再利用検知・行ロック付き回転** `+4`
  > 前回の弱点だった「回転が行ロックなしで競合する」が解消された。`refresh/1` はトランザクション内で `fetch_refresh_token_for_update`（`Ash.Query.lock("FOR UPDATE")`）を取り、失効済みトークンの再提示はトランザクションをロールバックしてから外側でファミリー全失効を行う（失効がロールバックに巻き込まれない）。同時 2 リクエストで成功が 1 件だけになることを `accounts_refresh_concurrency_test.exs` で固定しており、moduledoc で「共有 sandbox では真の競合ではなく API 契約の確認」と限界を正直に書いている点も良い。OAuth 2.0 Security BCP の rotation + reuse detection を実務水準で満たす。
  > 対象ファイル: `auth-server/lib/auth/accounts.ex`（L115-149, L322-330, L390-400）, `auth-server/test/auth/accounts_refresh_concurrency_test.exs`

- **RS256 + `kid`（JWK thumbprint）+ ローテーション用検証鍵** `+3`
  > 署名鍵と検証専用鍵を分けて JWKS に全鍵を載せ、`kid` 重複は起動時に例外で止める。prod は鍵の自動生成を禁止する。前回から変わらず堅い。
  > 対象ファイル: `auth-server/lib/auth/token/keys.ex`（L54-113）

- **多軸レート制限（IP × 識別子 × トークンファミリー）** `+3`
  > ETS の `update_counter` による固定窓。login は IP と identifier、refresh は IP とファミリーで二重に制限し、初回超過時だけ telemetry を発火する。
  > 対象ファイル: `auth-server/lib/auth/rate_limit.ex`, `auth-server/lib/auth_web/plugs/rate_limit.ex`

- **Tunnel 背後のクライアント IP 解決（信頼プロキシからのみ `CF-Connecting-IP` を採用）** `+2`
  > 前回 `-2` の「Tunnel 経由でレート制限が全員共有になる」を解消。`trusted_proxies` に一致する peer からの要求でだけ `CF-Connecting-IP` → `X-Forwarded-For` の順に読み、信頼外 peer のヘッダは無視する。edge の README に「Docker bridge 全体を信頼すると同一ネットの他コンテナが偽造できる」という具体的な落とし穴まで書かれている。テスト（`client_ip_test.exs`）もある。
  > 対象ファイル: `auth-server/lib/auth_web/client_ip.ex`（L4-50）, `edge/cloudflared/README.md`（L103-122）

- **Follow / Friendship のソーシャルグラフ（DB 制約付き）** `+2`
  > Friendship は `(sub_lo, sub_hi)` の正規化ペアで一意化し、マイグレーションで順序 CHECK と「requester がペアの一方であること」の CHECK を張っている。unfollow は Follow 削除と Friendship 解除を同一トランザクションで行う。相互フォロー時のみ申請可、accept は addressee のみ、と状態遷移が明確。アプリ層だけでなく DB 制約で不変条件を守る設計は良い。
  > 対象ファイル: `auth-server/lib/auth/social.ex`（L40-59, L69-130）, `auth-server/priv/repo/migrations/20261028030000_friendships_fk_and_order_check.exs`, `auth-server/priv/repo/migrations/20261028040000_friendships_requester_in_pair_check.exs`

- **アカウント列挙・タイミング攻撃への配慮** `+2`
  > ログイン失敗の文言統一、`Argon2.no_user_verify/0` による時間均一化、forgot-password の同一応答。
  > 対象ファイル: `auth-server/lib/auth/accounts.ex`, `auth-server/lib/auth/password.ex`

- **メール確認・パスワードリセットのトランザクション設計** `+2`
  > アカウントトークンを `FOR UPDATE` で消費し、リセット時はパスワード変更・トークン消費・refresh 全失効を同一トランザクションで行う。
  > 対象ファイル: `auth-server/lib/auth/accounts.ex`（L505-520 付近）

- **認証 Plug の失敗分類と情報を漏らさない構造化ログ** `+2`
  > 失敗理由を `missing_token / expired_token / invalid_token …` に分類してログに残し、応答はテンプレートのみ。当日のテストログでも `event: "auth.authenticate.failure"` が理由別に出ていた。
  > 対象ファイル: `auth-server/lib/auth_web/plugs/authenticate.ex`

- **S2S 内部トークンの prod fail-closed と定数時間比較** `+1`
  > `/internal/v1/access` は `x-alchemy-internal-token` を `Plug.Crypto.secure_compare` で照合し（長さ不一致は即 false）、未設定時は 503 を返す。prod では `AUTH_INTERNAL_TOKEN` が無いと起動時に raise し、既定値の秘密が本番に残らない。
  > 対象ファイル: `auth-server/lib/auth_web/plugs/internal_token.ex`（L12-49）, `auth-server/config/runtime.exs`（L78-86）

- **公開 / レート制限 / 認証必須 / 内部のパイプライン分離** `+1`
  > `:rate_limited`・`:authenticated`・`:internal` の 3 系統と、パイプラインなしの `/health`・JWKS が明確に分かれている。
  > 対象ファイル: `auth-server/lib/auth_web/router.ex`（L4-63）

### assets-server（CAS・Quota・門番）

- **メタと不変バイナリの分離 + BLAKE3 CAS の重複排除** `+2`
  > DB はメタと Quota のみ。同一ハッシュの再アップロードは CAS 書き込みも Quota 加算もしない。
  > 対象ファイル: `assets-server/lib/assets/blobs.ex`（L32-46）

- **CAS バックエンドの behaviour 抽象と起動時の必須 env 検査** `+2`
  > `Assets.CAS.Backend` の裏に Local / S3。s3 時に必須 env が欠ければ明示メッセージで落ちる。
  > 対象ファイル: `assets-server/lib/assets/cas/backend.ex`, `assets-server/config/runtime.exs`

- **s3 時の GET を presigned URL への 302 にする** `+2`
  > egress がサーバーを通らない⑦経路を最小コストで実現。
  > 対象ファイル: `assets-server/lib/assets/cas/s3.ex`, `assets-server/lib/assets_web/controllers/blob_controller.ex`（L33-36）

- **Quota を条件付き UPDATE 1 発で原子的に消費** `+2`
  > 前回 `-1` の「確認と加算が分離して競合する」を解消。`WHERE used_bytes + ? <= limit_bytes` 付きの `update_all(inc: …)` の影響行数で判定し、0 行ならロールバックする。台帳行は `on_conflict: :nothing` で先に確保する。教科書どおりの解法。
  > 対象ファイル: `assets-server/lib/assets/blobs.ex`（L48-52, L141-174）

- **`Plug.Parsers` 撤去による生 body 受信と、ExAws の HTTP を Req に差し替え** `+1`
  > 前回 `-2` の「JSON の Content-Type で空 Blob が黙って保存される」を解消（Endpoint に理由付きコメントで Parsers を置かない）。ExAws の `http_client` を自前の `Assets.CAS.ExAwsReq` にして hackney を依存から外し、前回検出した hackney の advisory も消えた（`mix.lock` に hackney なし）。
  > 対象ファイル: `assets-server/lib/assets_web/endpoint.ex`（L7-9）, `assets-server/lib/assets/cas/ex_aws_req.ex`, `assets-server/config/config.exs`（L26）

- **テスト環境で CAS 系 env を無視する隔離** `+1`
  > 前回 `-3` の「開発者の `.env` でテストが本番 R2 に接続する」を解消。runtime が test で CAS env を無視し、`test_env_isolation_test.exs` で固定、`ci.sh` でも `ASSETS_CAS_BACKEND=local` を強制する二重の防御。当日 13 件すべて成功した。
  > 対象ファイル: `assets-server/test/assets/test_env_isolation_test.exs`, `.scripts/ci.sh`（L120-124）

- **local CAS の原子的書き込み（tmp → rename）** `+1`
  > 対象ファイル: `assets-server/lib/assets/cas/local.ex`

### world-server（案内所・門番）

- **HostAttestation / ManifestSeal の発行（typ・aud を分離した RS256 JWT）** `+3`
  > host Room Token を持つ owner だけが、`typ=host_attestation` と `typ=manifest_seal`（`aud=alchemy-manifest-seal`）の JWT を取得できる。seal は body の blake3 だけを受け取り、world は entries を組まない（host が body を組み、world は署名だけ）。Room Token・attestation・seal が iss は同じでも typ / aud で取り違えられないよう分かれている。Untrusted host が配信する Manifest を「world がその時点で認めた内容」に縛る仕組みとして、個人プロジェクトでは珍しい水準。
  > 対象ファイル: `world-server/lib/world/host_attestation.ex`, `world-server/lib/world/manifest_seal.ex`, `world-server/lib/world_web/controllers/room_controller.ex`（L204-270, L320-392）

- **Room の永続化（Ash + SQLite）と、ストレージ障害を 503 に分離** `+2`
  > `World.Rooms` は `{:error, :storage_error}` と `:not_found` を厳密に区別し、真偽値ヘルパーをあえて持たない（ストレージ障害が `false` に潰れて認可を誤らないため、と moduledoc に理由がある）。Repo を止めたときに 503 `storage_unavailable` になることをテストで確認している。Postgres への切替手順も world-server.md に書かれている。
  > 対象ファイル: `world-server/lib/world/rooms.ex`（L1-21, L214-225）, `world-server/test/world/rooms_test.exs`（L55-66）

- **`host` ロールは owner のみ・トークン発行で部屋を作らない** `+2`
  > 前回 `-2` の「role を自己申告できる」を解消。`authorize_role_cached` で owner 以外の `host` 要求を 403 にし、未知 role は 422。トークン経路は `Rooms.get` だけで部屋を作らない。
  > 対象ファイル: `world-server/lib/world_web/controllers/room_controller.ex`（L131-199）

- **7 段階公開範囲の Token 発行時 ACL（auth への S2S・fail-closed）** `+2`
  > owner は常に許可、`public` は認証済みなら許可、`private` は拒否、`lan` は経路判定、ソーシャル系は auth の `/internal/v1/access` に問い合わせる。auth に届かなければ `:auth_unavailable` → 503 で、許可側に倒れない。発行済みトークンは TTL まで有効という非保証も moduledoc に明記。
  > 対象ファイル: `world-server/lib/world/room_access.ex`（L1-53）, `world-server/lib/world/auth_access.ex`

- **Room Token の claim 設計** `+2`
  > `sub / room_id / role / iss / aud / exp(300s) / jti`。auth の JWT とは iss / aud が分かれている。
  > 対象ファイル: `world-server/lib/world/token.ex`

- **auth JWT 検証と `sub` の転写（body の `sub` は無視）** `+2`
  > テストで `"ignored-body-sub"` が無視されることを固定。
  > 対象ファイル: `world-server/test/world_web/controllers/room_controller_test.exs`（L16-33）

- **`lan` 公開範囲の fail-closed 判定** `+1`
  > `CF-Connecting-IP` か `CF-Ray` があれば Tunnel 経由として常に拒否し、それ以外は RFC1918 / ループバック / ULA / リンクローカルのみ許可。
  > 対象ファイル: `world-server/lib/world_web/client_ip.ex`（L23-69）

- **匿名トークンを既定で OFF** `+1`
  > 対象ファイル: `world-server/config/runtime.exs`（L19-26, L88-92）

- **room_id 検証を protocol と共有ベクタで一致** `+1`
  > 前回 `-1` の「検証が 2 つある」を解消。protocol 側を `[A-Za-z0-9_-]`・64 byte に揃え、`validate_room_id_shared_vectors` テストに「Elixir 側と同期」と明記している。
  > 対象ファイル: `world-server/lib/world/room_id.ex`, `protocol/src/lib.rs`（L202-228, L657-680）

---

## 技術評価層 — 共通契約層

### protocol/（ワイヤ契約 SSoT）

- **チャネル責務の分離（Manifest / frame / presence / input / liveliness / snapshot）** `+3`
  > 低頻度の配置、高頻度のデルタ、Presence、Intent、liveliness、late join 用 snapshot を別キーに分けている。Resonite 系と同じ筋の設計。
  > 対象ファイル: `protocol/proto/*.proto`, `protocol/src/lib.rs`（L139-158）

- **Ed25519 によるワイヤ署名（Intent・Frame・Presence）** `+3`
  > `wire_crypto` に「`*_sig` を空にした encode に署名する」規約を集約し、host・client・e2e-probe が同じ関数を使う。Intent は client のセッション鍵、Frame / Presence は host 鍵で署名する。前回 `-3` の「盗聴した Room Token で 300 秒なりすませる」に対し、定常 Intent から JWT を外し、署名と単調 `intent_seq` で受理する方式に変えた。プロトコル層で署名規約を一箇所に固定している点が良い。
  > 対象ファイル: `protocol/src/wire_crypto.rs`, `protocol/proto/input_events.proto`（L28-34）

- **配送クラス契約（peer-delivery.md）と `decide_frame_tick`** `+3`
  > 「latest デルタ・欠落したら snapshot で再同期」という配送意味論を文書と関数の両方で固定した。`FRAME_TICK_GAP_RESYNC=1`、空デルタでは tick を進めない、snapshot キャッシュの tick は最後に publish した値、など境界条件まで明文化されている。Zenoh の best-effort 配送の上に整合性を作る設計として、同種 OSS でも説明できていないことが多い部分。
  > 対象ファイル: `protocol/peer-delivery.md`, `protocol/src/lib.rs`（L264-302, L693-721）

- **`PROTOCOL_PIN` をワイヤに載せ、不一致を検出** `+2`
  > 前回 `-2` の「PIN がログに出るだけ」を解消。`SceneManifest.protocol_pin` と `PresenceState.protocol_pin` に載り、client は不一致を検出する（ただし不一致時の挙動には弱点がある。マイナス点を参照）。
  > 対象ファイル: `protocol/proto/scene_manifest.proto`（L17-18）, `protocol/src/lib.rs`（L29-30, L260-263）

- **RenderFrame / SceneEntry の TRS（回転・スケール）と正規化ヘルパ** `+2`
  > 前回 `-2` の「RenderFrame に姿勢が無い」を解消。`Entity` と `SceneEntry` に quat xyzw と scale を追加し、proto3 のゼロ quat を identity に、非有限 scale を `(1,1,1)` に倒す正規化関数を protocol に置いた。「毒入り JSON が NaN フレームを生まない」とコメントで理由を書いている。
  > 対象ファイル: `protocol/proto/render_frame.proto`, `protocol/src/lib.rs`（L341-382）

- **role → capability の写像（`sim_capability`）** `+2`
  > Room Token の `role` を Intent / HTTP の能力ビットに写像し、host は bind 時に caps を凍結して Intent 適用前に `mask_intent` をかける。未知 role は caps 0 で bind 拒否。Elixir 側は crate を import できないので「表と lockstep」とコメントで対応を明示している。
  > 対象ファイル: `protocol/src/sim_capability.rs`, `client/host-server/src/session.rs`（L65-70）

- **client 外の独立クレート + protoc 同梱ビルド** `+2`
  > 対象ファイル: `protocol/build.rs`, `client/Cargo.toml`（L22）

- **キー式ヘルパと liveliness キーの `(sub, jti)` 解析** `+2`
  > 単セグメントの旧キーを拒否し、余分なセグメントも拒否する。単体テストあり。当日 protocol のテストは 20 件すべて成功した。
  > 対象ファイル: `protocol/src/lib.rs`（L304-334, L623-655）

- **`validate_room_id` を world と同じ規則に統一** `+1`
  > 対象ファイル: `protocol/src/lib.rs`（L178-228）

---

## 技術評価層 — ホストユーザー端末

### client/host-server（Untrusted 計算ホスト）

- **Intent セッション表（bind / steady / 単調 seq / 1 sub 1 セッション）** `+3`
  > bind は JWT + `session_pub` + 署名、steady は既知の `session_pub` + 前進した seq + 署名のみ受理する。署名検証を JWT 検証より先に行い（不正署名で JWKS を叩かせない）、同じ `session_pub` を別 `sub` に付け替えようとすると拒否する。テストはテスト内で RSA 鍵を生成して実際の JWT で bind 経路を通している。
  > 対象ファイル: `client/host-server/src/session.rs`（L43-115, L214-356）

- **world 定義の Scene から Rapier を構築（Manifest 駆動）** `+3`
  > 前回 `-3` の「定義と実行の逆転」の主因を解消。host は world の `GET …/scene` から `PhysicsWorld::from_entries` で剛体を組み、配置・半径・Grab 可否はすべて entries 由来になった。分類規則は `alchemy-shared-sim` の `classify_scene_entry` に一本化し、host と client が同じ関数を使う。
  > 対象ファイル: `client/host-server/src/main.rs`（L700-720, L813-821）, `client/shared-sim/src/lib.rs`（L33-88）, `world-server/lib/world/demo_scene.ex`

- **Presence 解放の三重化と、liveliness を検証済み jti に紐付け** `+3`
  > 前回 `-3` の「liveliness の Delete で誰でも他人を退室させられる」に対し、Delete のキーに含まれる `jti` が最後に検証した Intent の `jti` と一致するときだけ解放を予約し、猶予後に再確認してから解放する。refresh で jti が変わる場合の順序の入れ替わり（liveliness が Intent より先に届く）まで考慮してテストしている。
  > 対象ファイル: `client/host-server/src/main.rs`（L134-257, L1189-1240）

- **JWKS の SWR とバックグラウンド更新、未知 kid のクールダウン** `+2`
  > 前回 `-3` の「JWKS 更新を入力スレッドで同期実行」を解消。`verify` はキャッシュだけを見て HTTP を行わず、ソフト TTL 超過時はバックグラウンドを起こすだけ。未知 kid での再取得は 1 秒のクールダウン付き、ureq にはタイムアウトを設定。「キャッシュが空でも verify は HTTP しない」ことをテストで固定している。
  > 対象ファイル: `client/host-server/src/room_token.rs`（L24-31, L103-130, L166-243, L1038-1049）

- **move の単位球クランプ（f64 で長さ計算）と Grab 開始距離の判定** `+2`
  > 前回 `-2` の「有限の巨大値が通る」を大部分解消。`1e30` でも f32 の二乗が Inf にならないよう f64 で長さを計算し、Grab 開始は自分の user_root と箱の距離（client の pick 距離と同じ 5m を shared-sim で共有）で判定する。
  > 対象ファイル: `client/host-server/src/finite.rs`（L25-53）, `client/host-server/src/sim.rs`（L328-336）

- **snapshot queryable による純デルタ化と tick 規則** `+2`
  > 前回 `-1` の「途中参加者対策で毎 tick 全件 upsert」を解消。静止物は upsert せず、late join と tick 欠落は `…/snapshot` で取る。snapshot 未準備時は `reply_err` を返す。
  > 対象ファイル: `client/host-server/src/main.rs`（L586-619, L826-831）, `client/host-server/src/pose_delta.rs`

- **内部物理 60Hz と配信 30Hz の分離 + 固定ステップ** `+2`
  > 対象ファイル: `client/host-server/src/main.rs`（L823-849）

- **入力ゲートの順序と拒否理由ごとのカウンタ** `+2`
  > decode → セッション / Token → caps マスク → finite → クランプ → スロット割当。
  > 対象ファイル: `client/host-server/src/main.rs`（L480-583）

- **Room Token 検証の厳格さ（alg 固定・iss/aud/exp・room_id 束縛）** `+2`
  > 対象ファイル: `client/host-server/src/room_token.rs`（L166-215）

- **Rapier の SoA 構成と Grab の no-steal** `+2`
  > 対象ファイル: `client/host-server/src/sim.rs`（L286-345）

- **Mutex の poison を致命的エラーとして扱う** `+1`
  > 前回 `-1` の「poison を黙って空のプレゼンスに置き換える」を解消。`lock_or_fatal` で error ログを出して終了し、launcher の再起動に任せる。
  > 対象ファイル: `client/host-server/src/main.rs`（L34-40）

### client/app（体感・描画・UI エンドポイント）

- **Frame / Presence の署名検証と、seal + attestation の信頼チェーン** `+3`
  > Manifest の `world_seal`（body blake3 を world が署名）と `host_attestation`（host 公開鍵を world が証明）を world JWKS で検証してから `host_pub` を採用し、以降の Frame / Presence は `host_sig` が通らなければ破棄する。Frame には finite と tick の検査もある。「Untrusted host が配る描画データを、world が認めた鍵でしか受け付けない」チェーンを client 側で閉じている。
  > 対象ファイル: `client/app/src/main.rs`（L207-290, L445-500）, `client/app/src/manifest_seal.rs`, `client/app/src/host_attestation.rs`

- **system_ui が OpenXR / winit / Room を知らない** `+3`
  > 対象ファイル: `client/app/src/system_ui/mod.rs`, `client/app/src/system_ui/facet.rs`

- **OpenXR の実機コントローラ描画（render model・animatable node）** `+3`
  > 対象ファイル: `client/app/src/surface/xr/render_model.rs`

- **access / Room Token の 80% 自動更新と liveliness の付け替え** `+2`
  > 前回 `-3` の「入室 5 分後に入力がすべて捨てられる」を解消。`token-refresh` スレッドが access と Room Token を TTL の 80% で更新し、Room Token 更新後は次の Intent put の後に新しい `…/alive/{sub}/{jti}` を宣言して古い token を退役させる（host 側の jti 照合と順序を合わせている）。
  > 対象ファイル: `client/app/src/main.rs`（L802-905）, `client/app/src/auth.rs`（L36-111）

- **surface の自動選択とフォールバック** `+2`
  > 対象ファイル: `client/app/src/surface/mod.rs`

- **デコード前の BLAKE3 照合と 302 追従** `+2`
  > 対象ファイル: `client/app/src/main.rs`, `client/app/src/mesh.rs`

- **ログイン・入室の HTTP をワーカースレッドで実行** `+1`
  > 前回 `-1` の「UI スレッドで同期 HTTP」を解消。`auth-http` スレッドで実行し、結果を mpsc で返す。
  > 対象ファイル: `client/app/src/surface/desktop.rs`（L195, L936-1070）

- **新規ワールド作成とアクセスレベル選択（仮 UI）** `+1`
  > owner でない場合は既存 ACL のまま入室し、status で区別する。world-server.md に「門番ではない」と明記している。
  > 対象ファイル: `client/app/src/access_level.rs`, `client/app/src/world_api.rs`

- **Leave → liveliness undeclare の順序設計** `+1`
  > 対象ファイル: `client/app/src/main.rs`（L948-985）

- **サブルーター化しない純粋エンドポイント** `+1`
  > 対象ファイル: `client/app/src/main.rs`

### launcher/（プロセスマネージャ）

- **最小のプロセスマネージャ（ready ファイルでの IPC アドレス通知・終了理由の区別）** `+2`
  > 前回はスタブのみで評価対象外だった。host が `127.0.0.1:0` に自分で bind して ready ファイルにアドレスを書くので、launcher がポートを先取りする TOCTOU が無い。exit 0 は意図的停止として再起動せず、非 0 だけ指数バックオフで最大 3 回再起動する。IPC はワンタイムトークン付きでループバックのみ。launcher.md の「保証しない」表に「host 再起動後の client 側 Exit Client+Host」まで書かれているのは誠実。
  > 対象ファイル: `launcher/src/main.rs`（L56-160, L237-269）, `client/host-server/src/host_ipc.rs`, `protocol/src/host_ipc.rs`

---

## 技術評価層 — 転送・公開エッジ

### zenoh-router / edge

- **prod / release 前提の Tunnel 手順と `/dev/*` の ingress 遮断** `+2`
  > 前回 `-4` の「Tunnel 手順書が開発ビルドを公網へ公開する」を解消。3 サービスとも `docker-compose.prod.yml`（release ステージ）を用意し、手順書は prod のみを指示する。cloudflared の ingress で `auth/dev/*` を 404 にする多重防御、seed は `Mix.env() == :prod` でスキップ、Postgres は `127.0.0.1` のみに publish。（ただし prod compose に新たな欠陥がある。マイナス点を参照）
  > 対象ファイル: `edge/cloudflared/config.yml.example`（L15-20）, `edge/cloudflared/README.md`（L80-101）, `auth-server/docker-compose.prod.yml`, `auth-server/priv/repo/seeds.exs`（L11-12）

- **WAN の Zenoh を TLS 化し、接続ヘルパを共通化** `+2`
  > 前回 `-2` の「平文 TCP」のうち盗聴部分を解消。`config.wan.json5` は `tls/` のみを待ち受け、開発用の証明書生成スクリプトがある。host・client・e2e-probe は `zenoh-connect` クレートの同じ関数で `ZENOH_CONNECT` と `ZENOH_TLS_ROOT_CA` を適用する。
  > 対象ファイル: `zenoh-router/config.wan.json5`, `zenoh-router/scripts/gen-wan-dev-certs.sh`, `client/zenoh-connect/src/lib.rs`

- **ローカル / WAN 設定の分離と差分の明記** `+2`
  > 対象ファイル: `zenoh-router/config.wan.json5`（L1-4）, `zenoh-router/README.md`

- **WAN の運用手順（systemd・ufw allowlist）** `+1`
  > 対象ファイル: `.workspace/0_doc/manual/zenoh-wan-conoha.md`

- **「zenohd はトークンを見ない」という境界の明記** `+1`
  > 対象ファイル: `.workspace/0_doc/architecture/zenoh-router.md`, `.workspace/0_doc/architecture/trust-boundary.md`

- **`TRUSTED_PROXIES` の必須手順化** `+2`
  > peer IP の調べ方、信頼してはいけない範囲、未設定時の挙動までを手順書に書いた。
  > 対象ファイル: `edge/cloudflared/README.md`（L103-122）, `auth-server/.env.prod.example`

---

## 横断評価層

### テスト戦略

- **ヘルメティックな Compose 縦 E2E を GitHub Actions で実行** `+3`
  > 前回 `-2` の「サービスをまたぐ自動 E2E が無い」をおおむね解消。`docker-compose.e2e.yml` で auth / world / assets / zenohd / host を日常スタックとポートをずらして起動し、Rust 製の `e2e-probe` が login → Room Token → 署名付き Intent → user_root を含む RenderFrame までをアサートする。新しい smoke は「前提サービスが無ければ赤」と冒頭に明記している。
  > 対象ファイル: `.github/workflows/e2e-compose.yml`, `docker-compose.e2e.yml`, `client/e2e-probe/src/main.rs`, `.scripts/smoke-compose-e2e.sh`（L4）

- **auth-server の厚いテスト（124 件）** `+3`
  > 前回 107 件から増え、ソーシャル・S2S・ClientIp・並行 refresh が加わった。当日 Docker 上で format / compile --warnings-as-errors / credo --strict / hex.audit を含めすべて成功した。
  > 対象ファイル: `auth-server/test/**`

- **Rust のユニットテスト（client 86 / host 57 / shared-sim 3 / protocol 20）** `+3`
  > 前回の 81 件から 166 件へ。session・room_token の資格情報チェーンの計画関数・liveliness の猶予・pose delta・IPC がテストされている。テスト内で RSA 鍵を生成して本物の JWT で bind を通すなど、モックに逃げない作りが良い。当日 fmt / clippy -D warnings / build / test がすべて通過した。
  > 対象ファイル: `client/**/src/**`, `protocol/src/**`

- **Rust と Elixir の共有テストベクタ** `+1`
  > `validate_room_id` の受理・拒否ベクタを両言語のテストで同期させている。
  > 対象ファイル: `protocol/src/lib.rs`（L657-680）, `world-server/test/world/room_id_test.exs`

- **共有 JWKS キャッシュのテスト（時計を注入）** `+1`
  > `now_ms` を注入してクールダウンを決定的にテストしている。当日 9 件すべて成功した。
  > 対象ファイル: `shared/alchemy_auth_client/test/alchemy_auth_client/jwks_test.exs`

### 可観測性

- **auth の構造化失敗ログと telemetry** `+2`
  > 対象ファイル: `auth-server/lib/auth_web/plugs/authenticate.ex`, `auth-server/lib/auth/rate_limit.ex`

- **host の周期カウンタ** `+1`
  > 対象ファイル: `client/host-server/src/main.rs`（L1080-1110 付近）

### エラーハンドリング

- **Rust は anyhow + context、Elixir はタグ付きタプル + FallbackController** `+2`
  > 起動失敗に「auth → host Room Token → HostAttestation」のような段階名の文脈が付く。
  > 対象ファイル: `client/host-server/src/main.rs`（L688-704）

- **ストレージ障害と「存在しない」の分離** `+2`
  > `World.Rooms` が `:storage_error` を 503 に、`:not_found` を 404 に分け、障害を 404 / 422 / false に潰さない方針を明文化している。認可判断がインフラ障害で誤らない設計。
  > 対象ファイル: `world-server/lib/world/rooms.ex`（L14-21）, `world-server/lib/world_web/controllers/room_controller.ex`（L414-418）

### 変更容易性・保守性

- **保証単位のファイル分割と、モジュールごとの Guarantees / Non-guarantees** `+3`
  > 新規ファイル（session.rs・host_ipc.rs・room_access.ex・social.ex 等）にも保証と非保証が書かれ、しかも弱点を自ら非保証に書いている（例: session.rs「Token eavesdropping on bind」、finite.rs「aim still drives hold offset」）。TODO / FIXME はコード内に 0 件。
  > 対象ファイル: `client/host-server/src/session.rs`（L1-7）, `client/host-server/src/finite.rs`（L1-5）ほか

- **JWKS キャッシュと JWT 検証を `alchemy_auth_client` に共通化** `+2`
  > 前回 `-2` の「world と assets でコピーされている」を解消。path 依存の小さな mix パッケージにし、各サービスは名前付きの facade だけを持つ。専用 CI もある。
  > 対象ファイル: `shared/alchemy_auth_client/lib/**`, `world-server/lib/world/auth/jwks.ex`, `.github/workflows/alchemy-auth-client-ci.yml`

- **Scene 分類の単一正本（`alchemy-shared-sim`）** `+2`
  > host と client が別々に持っていた分類規則・Grab 距離・既定半径を 1 クレートに集約した。
  > 対象ファイル: `client/shared-sim/src/lib.rs`

- **`undocumented_unsafe_blocks = "deny"` による unsafe 根拠の強制** `+1`
  > 前回 `-2` の「unsafe 81 か所に SAFETY 約 10 か所」を仕組みで解消した（workspace lints で全メンバーに適用）。ただしコメントの多くは定型文で、個々の不変条件までは書いていない。
  > 対象ファイル: `client/Cargo.toml`（L16-18）, `client/app/src/surface/xr/vk_loop.rs`

### 開発者体験（DX）

- **README → WSL セットアップ → compose → cargo run、`make ci` を単一入口に** `+3`
  > 対象ファイル: `README.md`, `Makefile`, `.scripts/ci.sh`

- **3 サービスとも dev / release の 2 ステージ Docker と prod compose** `+2`
  > 対象ファイル: `*/Dockerfile`, `*/docker-compose.prod.yml`

- **Rust CI に `Swatinem/rust-cache` を導入** `+1`
  > 前回 `-1` を解消。
  > 対象ファイル: `.github/workflows/rust-ci.yml`

### セキュリティ・配布可能性

- **Untrusted 入力の入口防御（署名 → JWT → caps → finite → クランプ）** `+2`
  > 対象ファイル: `client/host-server/src/main.rs`（L480-583）, `client/host-server/src/finite.rs`

- **release ビルドから既定資格情報を除去（fail-closed）** `+2`
  > 前回 `-1` を解消。`alice / Password1` は `#[cfg(debug_assertions)]` の下にだけ存在し、release で env が無ければエラー。「release で seed 許可フラグを拒否する」テストもある。
  > 対象ファイル: `client/host-server/src/room_token.rs`（L40-44, L362-420, L1051-1092）

- **Zenoh `host_control` を既定 OFF、launcher はローカル IPC** `+2`
  > 前回 `-2` の「shutdown が無認証」を解消。既定では購読しない。launcher 経路はループバック + トークン。
  > 対象ファイル: `client/host-server/src/host_control.rs`（L15-23）, `client/host-server/src/host_ipc.rs`

- **依存の脆弱性管理（cargo deny を CI でゲート、hex.audit）** `+1`
  > 前回 `-1` をおおむね解消。Rust は `cargo-deny-action` で必須、Elixir は `mix hex.audit`（ただし CVE 照合ではない。提案を参照）。
  > 対象ファイル: `.github/workflows/rust-ci.yml`（L44-56）, `client/deny.toml`

### プロジェクト全体設計

- **architecture/*.md の「保証する／しない」表がコードと一致している** `+4`
  > protocol.md の PIN（`0.15.0-x002e`）、client.md のキー一覧、world-server.md の API 表と公開範囲、launcher.md の終了境界が、いずれも当日コードと一致した。前回空だった launcher.md も実装に合わせて書かれ、zenoh-router.md のキー表も補完された。
  > 対象ファイル: `.workspace/0_doc/architecture/*.md`

- **信頼境界の脅威モデル（trust-boundary.md）** `+3`
  > Zenoh キー族ごとに「偽 publish の被害 / 現状の防御 / 保証しない」を 1 表にまとめ、HTTP 門番・ホスト端末プロセス境界も同じ形式で整理した。各行が実装スライスに対応している。個人プロジェクトで STRIDE 風の対応表をコードと同期して保守している例は少ない。（表のうち manifest 行の「失敗は leave」が新しい攻撃面になっている点はマイナス点を参照）
  > 対象ファイル: `.workspace/0_doc/architecture/trust-boundary.md`

- **縦スライスのロードマップ規律** `+3`
  > 前回評価の弱点を P0〜P4 として番号付きスライスに落とし、1 スライス 1 PR で処理した（x001e〜x002f）。評価 → 改善計画 → スライス → PR の自己改善サイクルが実際に回っている。
  > 対象ファイル: `.workspace/roadmap/README.md`, `.workspace/roadmap/slices/`

- **旧 Elixir 権威実行からの転換** `+2`
  > world-server は Pure Elixir（NIF・独自 UDP・空間 Channel なし）のまま、空間計算は Rust の host にある。
  > 対象ファイル: `world-server/mix.exs`, `world-server/lib/**`

- **コンポーネント単位でパスフィルタした CI** `+1`
  > 対象ファイル: `.github/workflows/*.yml`
