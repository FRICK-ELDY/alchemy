# Alchemy — マイナス点詳細（第1評価者 Claude Opus 5.5 / 2026-10-26）

> 評価対象コミット: `683e30f`（main）。全項目は当日コードを直接読んで確認した。
> 第2評価者の文書は参照していない。各項目に改善方針を付ける。

## 採点基準

| 点数 | 基準 |
|:---:|:---|
| -1 | 改善余地あり。動作はするが設計・品質上の軽微な問題 |
| -2 | 重要な機能・設計の欠如。放置すると将来の拡張を阻害する |
| -3 | 設計上の明確な欠陥。バグ・クラッシュ・性能劣化を引き起こしうる |
| -4 | プロジェクトの価値命題を損なう重大な欠如。説明責任が果たせない |
| -5 | プロジェクトの根幹を揺るがす致命的な欠陥。存在しないに等しい |

---

## 技術評価層 — 運営側コア

### auth-server

- **Tunnel 経由ではクライアント IP が 1 つに潰れ、レート制限が全員共有になる** `-2`
  > `ClientIp.from_conn/1` は `trusted_proxies == []` のとき TCP の送信元 IP をそのまま使う。x001a の Tunnel 構成では、cloudflared から Docker へ転送されるため、全利用者が同じ送信元 IP（Docker ゲートウェイ）に見える。その結果、login の IP 軸の上限（prod 既定で 30 回/分）が全ユーザー共通になり、攻撃者 1 人で全員のログインを止められる。`TRUSTED_PROXIES` は edge の手順書に一切出てこず、Cloudflare が付ける `CF-Connecting-IP` も読んでいない。
  > 改善: edge の README に `TRUSTED_PROXIES` の設定を必須手順として追加する。`CF-Connecting-IP` を優先して読む（信頼できるプロキシ経由のときに限る）。
  > 対象ファイル: `auth-server/lib/auth_web/client_ip.ex`（L4-23）, `auth-server/config/runtime.exs`（L125-132）, `edge/cloudflared/README.md`

- **refresh の回転が行ロックなしで競合する** `-1`
  > `refresh/1` はトランザクションの外で refresh token のレコードを読み、`do_refresh/2` のトランザクション内でも `FOR UPDATE` を取っていない。同じ refresh token で同時に 2 リクエストが来ると、両方が「未失効」と判定して新しいトークンを 2 本発行し、再利用検知をすり抜ける。アカウントトークン側（`fetch_account_token`）では `FOR UPDATE` を使っているので、扱いが一貫していない。
  > 改善: `get_by_token_hash` を `Ash.Query.lock("FOR UPDATE")` 付きでトランザクション内に移す。または `revoke` を `WHERE revoked_at IS NULL` 付きの条件更新にして、影響行数が 0 なら失敗にする。
  > 対象ファイル: `auth-server/lib/auth/accounts.ex`（L114-134, L307-328）

- **署名鍵の取得が GenServer.call で直列化される** `-1`
  > 発行・検証のたびに `Keys.signer/0` / `signer_for_kid/1` が 1 プロセスへの call になる。負荷が上がると auth 全体のボトルネックになる。
  > 改善: 鍵の状態を `:persistent_term` に置き、ローテーションのときだけ書き換える。
  > 対象ファイル: `auth-server/lib/auth/token/keys.ex`（L16-24）

### assets-server

- **JSON / フォーム / multipart の Content-Type でアップロードすると、空の Blob が黙って保存される** `-2`
  > Endpoint の `Plug.Parsers` が `:urlencoded, :multipart, :json` を先に消費するため、`BlobController.create/2` の `read_body/2` には空の body しか届かない。たとえば `.gltf`（JSON）や JSON の Manifest を `application/json` で PUT すると、0 byte の BLAKE3 が `201` で返り、エラーにならない。テストは `text/plain` と `application/octet-stream` しか試していない。
  > 改善: `/api/v1/blobs` だけ Parsers を通さない（Router のパイプラインで `Plug.Parsers` を外す、または `body_reader` で生 body を退避する）。JSON の Content-Type でのテストを追加する。
  > 対象ファイル: `assets-server/lib/assets_web/endpoint.ex`（L7-11）, `assets-server/lib/assets_web/controllers/blob_controller.ex`（L6-23）

- **BLAKE3 を外部 CLI と一時ファイルで計算している** `-1`
  > アップロードのたびに body を `System.tmp_dir!()` へ既定の権限（0644）で書き出し、`b3sum` を起動する。100MB 上限のファイルなら、メモリ上に 1 回、tmp に 1 回、CAS に 1 回と 3 回コピーされる。共有の tmp に他人の資産が一時的に読める状態で置かれる。moduledoc で「Rustler の不安定さを避けるため」と理由を書いている点は良い。
  > 改善: 当面は stdin へパイプする（`Port.open` に `{:spawn_executable, b3sum}` と `-` を渡す）。将来は rustler_precompiled 版の blake3 か、ストリーミングで計算しながら CAS の tmp に直接書く方式にする。
  > 対象ファイル: `assets-server/lib/assets/blake3.ex`（L9-33）

- **Quota の確認と加算が分離していて競合する** `-1`
  > `ensure_quota!/2` で読んでから、`add_quota_usage!/2` で `inc` している。同時アップロードでは両方が確認を通過し、上限を超えうる。
  > 改善: `UPDATE … SET used_bytes = used_bytes + ? WHERE owner_sub = ? AND used_bytes + ? <= limit_bytes` のような条件付き更新にし、影響行数で判定する。
  > 対象ファイル: `assets-server/lib/assets/blobs.ex`（L140-168）

- **GET の認可モデルが文書化されていない** `-1`
  > `BlobRef` の主キーは blake3 だけで、`owner_sub` は最初にアップロードした人しか記録されない。GET は認証済みであれば誰でも任意のハッシュを取得できる。ハッシュを知っていればファイルの存在確認にもなる。overview の「見る権利は全員即時 OK」と整合する設計判断かもしれないが、assets-server.md の「保証しない」表にも書かれていない。
  > 改善: 「Blob は capability（ハッシュを知る者は読める）」を保証表に明記する。将来の Vault / License に向けて、所有者リンク表（`owner_sub × blake3`）を別テーブルに分けておく。
  > 対象ファイル: `assets-server/lib/assets/blobs/blob_ref.ex`, `assets-server/lib/assets_web/controllers/blob_controller.ex`（L25-48）

### world-server

- **`role` を呼び出し側が自己申告でき、部屋の所有権もない** `-2`
  > `create_token/2` は `params["role"]` をそのまま使う。認証済みのユーザーなら誰でも、任意の部屋の `host` Room Token を取得できる。`Rooms.ensure/1` は存在しない部屋を自動で作る。現状 host-server は role で権限を分けていないため実害は小さいが、verifier やホスト移譲が role を前提にすると、この穴がそのまま権限昇格になる。
  > 改善: Room に `owner_sub` を持たせ、`host` は owner（または委任された sub）にだけ発行する。`ensure` による自動作成は guest トークンでは行わない。
  > 対象ファイル: `world-server/lib/world_web/controllers/room_controller.ex`（L28-29, L56-83）, `world-server/lib/world/rooms.ex`（L60-68）

- **未知の `kid` が来るたびに、GenServer の中で JWKS を同期で再取得する** `-2`
  > `signer_for_kid/1` で未知の kid が来ると、GenServer の call の中で `Req.get`（2 秒のタイムアウト）を実行する。クールダウンが無いため、ランダムな kid を持つトークンを連投すれば、world から auth への HTTP が攻撃者の送信数だけ発生し、その間はすべての検証が直列に待たされる。assets-server にも同じコードがある。
  > 改善: 再取得に最小間隔（例: 30 秒）を設け、間隔内の未知 kid は即座に拒否する。HTTP 取得は Task に切り出し、GenServer を塞がない。
  > 対象ファイル: `world-server/lib/world/auth/jwks.ex`（L64-75, L101-116, L126-140）, `assets-server/lib/assets/auth/jwks.ex`

- **Room 一覧が上限なしで増え続ける** `-1`
  > `ensure` は呼ばれるたびに `MapSet.put` する。削除も TTL も無いので、認証済みユーザーがランダムな room_id で叩き続けるとメモリが増え続ける。
  > 改善: ユーザーごとの作成数上限と、最終トークン発行からの TTL を設ける。
  > 対象ファイル: `world-server/lib/world/rooms.ex`（L44-68）

- **テストが薄い（6 件）** `-1`
  > 期限切れ・aud の不一致・inactive ユーザー・未知 kid での再取得・匿名フラグ ON/OFF といった検証の否定経路が、いずれもテストされていない。門番サービスとしては最も厚くテストすべき部分。
  > 改善: `World.Auth.Verify` の否定系テーブルテストと、JWKS の再取得・クールダウンのテストを追加する。
  > 対象ファイル: `world-server/test/`

---

## 技術評価層 — 共通契約層

### protocol/

- **`PROTOCOL_PIN` がワイヤに載っておらず、起動時のログに出るだけ** `-2`
  > `PROTOCOL_PIN` は定数として定義され、host と client の起動バナーに出力されるだけで、どのメッセージにも含まれていない。世代の違う host と client が接続しても、それを検出する手段がない（protobuf は未知のフィールドを黙って無視する）。smoke も「バナーに文字列があるか」を grep しているだけ。
  > 改善: `SceneManifest` と `PresenceState` に `protocol_pin` フィールドを追加する（または Zenoh の attachment に載せる）。client は不一致なら入室を拒否し、UI に表示する。
  > 対象ファイル: `protocol/src/lib.rs`（L18-19）, `.scripts/smoke-leave-liveliness.sh`（L52-55）

- **RenderFrame に姿勢（回転）が無い** `-2`
  > `Entity` は `id, x, y, z, rgba` だけを持つ。host の Rapier では箱が転がって回転するのに、client は軸に揃った箱として描くので、物理と見た目がずれる。VR 基盤で回転を持たない姿勢ストリームは、アバターやコントローラを載せる段階で必ず破綻する。
  > 改善: `qx, qy, qz, qw`（float、smallest-three 圧縮は後で）を追加し、PIN を上げる。
  > 対象ファイル: `protocol/proto/render_frame.proto`（L20-31）, `client/host-server/src/sim.rs`（L81-91）

- **`validate_room_id` が world-server の検証と一致しない** `-1`
  > protocol 側は「空でない・`/` を含まない・印字可能な ASCII」だけを見るため、Zenoh で特別な意味を持つ `*` `$` `?` `#` やスペースが通り、長さの上限も無い。world-server は `^[A-Za-z0-9_-]+$` で最大 64 byte。同じ概念に対して検証が 2 つある。
  > 改善: protocol を正本にして同じ正規表現と長さ上限にそろえ、world-server からも同じ規則を参照する（規則を文書化し、両方のテストで同じベクタを使う）。
  > 対象ファイル: `protocol/src/lib.rs`（L74-89）, `world-server/lib/world/rooms.ex`（L75-92）

---

## 技術評価層 — ホストユーザー端末

### client/host-server

- **JWKS の更新を入力スレッドで同期実行するため、world が落ちると 60 秒後に全入力が止まる** `-3`
  > `verify/1` は毎回 `ensure_fresh_jwks()` を呼び、キャッシュが 60 秒より古ければ `refresh_jwks()`（タイムアウト指定なしの ureq）を実行し、失敗すると `?` でエラーを返す。失敗時は `fetched_at` を更新しないので、その後はパケットのたびに HTTP を試みる。その結果、次のことが起きる。
  > (1) world-server が一時的に落ちると、鍵はキャッシュにあるのに 60 秒後から全員の入力が `Room Token invalid` で捨てられる。
  > (2) 入力スレッドが HTTP で無期限に止まりうる。
  > (3) 未知の kid を含むトークンを送るだけで、host に world への HTTP を打たせられる。
  > 改善: 鍵の更新はバックグラウンドのスレッドで行い、失敗しても古い鍵を使い続ける（stale-while-revalidate）。未知 kid による再取得にはクールダウンを設け、ureq にタイムアウトを設定する。
  > 対象ファイル: `client/host-server/src/room_token.rs`（L67-95, L97-150）

- **入力の大きさを制限せず、Grab の距離も確認しない** `-2`
  > `is_finite` は NaN と Inf だけを落とすので、`move_x = 1e30` のような有限の巨大値は通る。`mx * speed * dt` は f32 でオーバーフローして Inf になり、kinematic body の座標が発散する。`aim_*` にも制限が無いので、どこからでも任意の箱を掴んで任意の座標へ動かせる。overview が謳う「物理発散爆弾を 100% 遮断」は、現状のコードでは成立していない（finite.rs 自身も「magnitude clamps は保証しない」と書いている）。
  > 改善: `move_*` は長さを 1 にクランプする。`aim` は自分の user_root からの距離（例: 3m 以内）で棄却し、Grab 開始時にも距離を確認する。
  > 対象ファイル: `client/host-server/src/finite.rs`（L1-16）, `client/host-server/src/sim.rs`（L238-277, L347-370）

- **途中参加者対策として、箱を毎 tick 全件 upsert している** `-1`
  > 「デルタ配信」のはずが、箱 9 個は静止していても毎回送られる。コメントにも途中参加者のための回避策と書いてある。
  > 改善: Zenoh の queryable で `…/snapshot` を提供し、入室時に 1 回だけ全量を取得させる。frame は純粋なデルタに戻す。
  > 対象ファイル: `client/host-server/src/main.rs`（L702-711）

- **main.rs に約 300 行の単一 `main` があり、publish の失敗でプロセスごと終了する** `-1`
  > 起動・トークン取得・Zenoh の宣言・物理ループ・デルタ計算・ログがすべて `main()` にある。frame / presence / manifest の `put` が一度でも失敗すると `?` で host が終了する。
  > 改善: `HostLoop` 構造体に分け、一時的な publish 失敗は warn してカウンタを増やすだけにする。
  > 対象ファイル: `client/host-server/src/main.rs`（L460-770, L587-591, L631-637, L731-734）

### client/app

- **Room Token を更新しないため、入室から 5 分後に入力がすべて捨てられる** `-3`
  > world が発行する Room Token の TTL は 300 秒（`room_token_ttl_seconds: 300`）。client は入室時に 1 回だけ取得し（`join_room_with_access`）、入力を publish するスレッドは同じトークンを送り続ける。300 秒を過ぎると host は `exp` で全入力を拒否し、さらに 3 秒後には Intent TTL でスロットが回収される。その結果、ユーザーは操作できなくなり、自分のアバターも消える。`auth::refresh` も `#[expect(dead_code)]` のままで、access token（900 秒）の更新も無い。
  > 改善: `expires_in` の 80% の時点で Room Token を再取得するタイマーを置く。access token も refresh token で更新する。少なくとも、期限切れを UI に表示して再入室させる。
  > 対象ファイル: `client/app/src/main.rs`（L400-411, L356-359）, `client/app/src/auth.rs`（L129-141）, `world-server/config/config.exs`（L5）

- **`unsafe` が 81 か所あるのに、SAFETY コメントは約 10 か所しかない** `-2`
  > 実装ルールでは「`unsafe` を使う場合は必ずコメントで安全性の根拠を説明する」と定めている。`vk_loop.rs`（45 か所）と `render_model.rs`（30 か所）のほとんどの `unsafe { device.create_* }` には根拠が書かれていない。
  > 改善: 生存期間とスレッド所有権の不変条件を構造体レベルにまとめて記述し、各 `unsafe` からそれを参照する。clippy の `undocumented_unsafe_blocks` を `deny` にする。
  > 対象ファイル: `client/app/src/surface/xr/vk_loop.rs`, `client/app/src/surface/xr/render_model.rs`

- **描画経路が 2 本ある（Desktop は wgpu、XR は ash の生 Vulkan）** `-2`
  > XR は 2,503 行の `vk_loop.rs` で、パイプライン・デスクリプタ・ステージングバッファ・テクスチャを手書きし、WGSL は naga で multiview の SPIR-V に変換している。Desktop は wgpu。メッシュ・箱・UI を描くたびに 2 か所を直す必要があり、overview の「単一 Pure Rust コードベースから Web / Quest へ」とも逆方向の負債になる。Bevy（bevy_openxr）のように wgpu の上に XR を載せる方式が一般的。
  > 改善: wgpu-hal の Vulkan interop（`create_texture_from_hal`）で OpenXR の swapchain image を wgpu テクスチャとして包み、描画は `scene.rs` に一本化する。当面は `vk_loop.rs` を pipeline / resources / frame に分割する。
  > 対象ファイル: `client/app/src/surface/xr/vk_loop.rs`, `client/app/src/scene.rs`

- **ログイン中に UI スレッドで同期 HTTP を実行する** `-1`
  > Desktop の Home タブでは、`auth::login` → `join_room_with_access` を winit のイベント処理の中で同期的に呼ぶ。ureq にタイムアウトも無い。auth が遅いと描画が止まる。
  > 改善: ワーカースレッドで実行し、結果をチャネルで返す。ステータス表示はすでにあるので、それを活かす。
  > 対象ファイル: `client/app/src/surface/desktop.rs`（L699-770）

- **登録時に利用規約の同意を自動で送っている** `-1`
  > `register` は `"tos_agreed": true` を固定で送るため、ユーザーは規約を見ていない。auth 側は `stamp_tos_agreement` で同意を記録しているので、同意の記録が実態と食い違う。
  > 改善: Home の登録フォームに規約の版と同意チェックボックスを置き、同意した版を送る。
  > 対象ファイル: `client/app/src/auth.rs`（L83-105）

---

## 技術評価層 — 転送・公開エッジ

### edge / zenoh-router

- **Tunnel の手順書が開発ビルドを公網へ公開し、メールのプレビュー経由で任意アカウントを乗っ取れる** `-4`
  > edge の README（§5）は「拠点サービス起動」として各サーバーの `docker compose up -d` を指示しているが、この compose は `target: dev` / `MIX_ENV: dev` で起動する。その結果、次の状態がインターネットに公開される。
  > - `dev_routes: true` により、`https://auth.<zone>/dev/mailbox`（Swoosh の MailboxPreview）と `/dev/dashboard`（LiveDashboard）が認証なしで見える
  > - `debug_errors: true` により、例外時にスタックトレースとソースが表示される
  > - seed アカウント（`alice` / `bob` / `carol` / `admin`、パスワード `Password1`）がある
  > - レート制限の上限が 300 に緩められている
  >
  > 攻撃の連鎖: 被害者のメールアドレスで `/forgot-password` を叩き、`/dev/mailbox` で再設定トークンを読み、`/reset-password` を呼べば、任意のユーザーを乗っ取れる。auth-server 自体の実装品質は高いが、公開経路の設定でそれが無効になっている。「拠点 IP 秘匿」の価値命題以前の問題。
  > 改善: `docker-compose.prod.yml`（auth は既存の `release` ステージ、world / assets も release 化）を用意し、Tunnel の手順はそれだけを前提にする。どうしても dev を公開するなら、Cloudflare Access で全体に認証をかけ、`/dev/*` を ingress で 404 にする。
  > 対象ファイル: `edge/cloudflared/README.md`（L78-91）, `auth-server/docker-compose.yml`（L24-56）, `auth-server/config/dev.exs`（L20-28, L54）, `auth-server/lib/auth_web/router.ex`（L43-57）, `auth-server/priv/repo/seeds.exs`

- **WAN の Zenoh が平文の TCP で、TLS も ACL も無い** `-2`
  > `config.wan.json5` は `tcp/0.0.0.0:7447` だけを待ち受ける。Room Token（Bearer）、Presence に含まれる `sub`（ユーザー UUID）、入力のすべてが、インターネット上を平文で流れる。ufw の IP allowlist は送信元を絞るだけで、盗聴は防げない。zenohd は TLS / QUIC と mTLS、キー式ごとの ACL を標準で持っている。
  > 改善: `tls/0.0.0.0:7447`（または `quic/`）に切り替え、Let's Encrypt の証明書を使う。`access_control` で `alchemy/dev/room/*/input` への publish を client だけに許可するなど、最低限の ACL を入れる。
  > 対象ファイル: `zenoh-router/config.wan.json5`（L10-14）

---

## 横断評価層

### テスト戦略

- **assets のテストが開発者の `.env` で上書きされ、実際の R2 に接続する（ローカル CI が失敗）** `-3`
  > `config/runtime.exs` は環境を区別せずに `ASSETS_CAS_BACKEND` と `ASSETS_S3_*` を適用する。そのため、`test.exs` の `cas_backend: :local` が上書きされる。docker-compose は `assets-server/.env`（gitignore 済みで、R2 の実際の認証情報が入っている）を読むので、`make ci-assets` を実行するとテストが本番バケットに PUT し、GET で 302 が返って `upload download and dedupe quota` が失敗した（当日 7 件中 1 件が失敗）。GitHub Actions には `.env` が無いので通るが、手元と CI で結果が違う。さらにテストのたびに本番ストレージへ書き込みが発生する。
  > 改善: runtime.exs の CAS 関連は `if config_env() != :test` で囲む。CI スクリプトでは `docker compose run` に `--env-file /dev/null` を付けるか、`-e ASSETS_CAS_BACKEND=local` を強制する。S3 のテストは ExAws をモックするか MinIO コンテナで行う。
  > 対象ファイル: `assets-server/config/runtime.exs`（L14-68）, `assets-server/config/test.exs`（L14-21）, `assets-server/docker-compose.yml`（L18-26）, `.scripts/ci.sh`（L105-116）

- **サービスをまたぐ自動 E2E が無く、smoke はスキップしても成功を返す** `-2`
  > login → Room Token → Zenoh → host → frame という縦の流れを、CI で自動的に検証するものが無い。smoke スクリプトは前提のサービスが無いと「ok (units only)」で `exit 0` し、成功判定の多くはログのバナー文字列の grep（例: `presence_ttl_secs=3`）で、挙動を確かめていない。
  > 改善: GitHub Actions で compose（auth / world / assets / zenohd）とヘッドレスの host を起動し、Rust のテスト用 client（Zenoh で input を送り frame を受け取る）で「Leave で解放されること」「不正トークンが拒否されること」をアサートする。smoke でのスキップは `exit 77`（skip）にして、成功と区別する。
  > 対象ファイル: `.scripts/smoke-*.sh`（例: `smoke-leave-liveliness.sh` L17-44）, `.github/workflows/`

- **host の Room Token 検証に、成功系・失敗系のテストが無い** `-1`
  > テストは「空のトークンを拒否する」の 1 件だけ。room_id の不一致・期限切れ・別の鍵・`alg` の差し替えといった、入室ゲートで最も重要な分岐が固定されていない。
  > 改善: テスト内で RSA 鍵を生成し、`KeyCache` を直接注入できるコンストラクタを用意して、表形式でテストする。
  > 対象ファイル: `client/host-server/src/room_token.rs`（L220-229）

### 可観測性

- **メトリクスの出力・トレース・サービス間の相関 ID が無い** `-2`
  > Elixir 側の `telemetry.ex` は Phoenix 既定のままで、exporter が無い。Rust 側は env_logger による文字列ログだけ。host のカウンタも数値としては外に出ない。auth → world → host と流れるリクエストを横断して追う ID が無く、本番でのデバッグは各ログの grep に頼るしかない。
  > 改善: Elixir は `telemetry_metrics_prometheus`（または PromEx）、Rust は `tracing` と OTLP に移行する。Room Token の `jti` を相関 ID として host のログにも出す。
  > 対象ファイル: `*/lib/*_web/telemetry.ex`, `client/host-server/src/main.rs`

### エラーハンドリング

- **500 応答に `inspect(reason)` で内部情報を載せている** `-1`
  > world の `token_issue_failed` と、assets の `upload_failed` / `read_body_failed` が、内部エラーの構造（Ecto の changeset や ExAws のエラーなど）をそのまま JSON で返す。auth は汎用メッセージにしているので、3 サービスで方針がそろっていない。
  > 改善: 詳細はログにだけ出し、応答はエラーコードのみにする。auth の方針に合わせる。
  > 対象ファイル: `world-server/lib/world_web/controllers/room_controller.ex`（L78-81）, `assets-server/lib/assets_web/controllers/blob_controller.ex`（L17-21, L76-79）

- **Mutex が poison されると、黙って「空のプレゼンス」に置き換える** `-1`
  > 物理ループは `presence.lock()` に失敗すると、全スロットが空の既定値を使って処理を続ける。入力スレッドが panic しても気づけず、全員が消えたように見える。
  > 改善: poison は致命的なエラーとして error ログを出して終了する（launcher が再起動する前提）。または `into_inner()` で中身を回復し、そのことをログに残す。
  > 対象ファイル: `client/host-server/src/main.rs`（L600-623）

### 変更容易性・保守性

- **JWKS キャッシュと JWT 検証のコードが world と assets でコピーされている** `-2`
  > `World.Auth.Jwks` と `Assets.Auth.Jwks` はモジュール名を除くと 7 行しか違わず、`Verify` は完全に同じ。セキュリティ上重要なコードが 2 か所にあるので、上で挙げた kid の再取得の穴も 2 か所にある。
  > 改善: `alchemy_auth_client`（path 依存の小さな mix パッケージ）に切り出し、両サービスから使う。
  > 対象ファイル: `world-server/lib/world/auth/{jwks,verify}.ex`, `assets-server/lib/assets/auth/{jwks,verify}.ex`

- **リポジトリ直下に誤ってコミットされた `-o` ファイル** `-1`
  > 中身は白い PNG 画像（x0018 の `045f4e7` で混入）。`curl -o` の引数の誤りと思われる。
  > 改善: `git rm -- -o` で削除する。
  > 対象ファイル: `-o`

### 開発者体験（DX）

- **`make ci` が手元で通らない（.env がある開発者の環境）** `-1`
  > 原因はテスト戦略で挙げた runtime.exs の問題と同じ。「ローカル CI がエラーなしで通ること」を前提にした運用が、R2 を設定した時点で崩れる。
  > 対象ファイル: `.scripts/ci.sh`（L105-116）

- **GitHub Actions の Rust CI にキャッシュが無く、3 ジョブがそれぞれフルビルドする** `-1`
  > fmt/clippy、build、test が別ジョブで、どれも `Swatinem/rust-cache` を使っていない。zenoh・wgpu・rapier・openxr のビルドを毎回 3 回行う。
  > 改善: rust-cache を導入し、build と test を 1 つのジョブにまとめる。
  > 対象ファイル: `.github/workflows/rust-ci.yml`

### セキュリティ・配布可能性

- **liveliness の Delete で、誰でも他人を退室させられる** `-3`
  > host は `…/alive/{sub}` の Delete を受け取ると、トークンを確認せずにその `sub` のスロットを即座に解放する。`sub` は PresenceState で全員に公開されているので、Zenoh に接続できる人なら誰でも「被害者の `sub` で liveliness トークンを宣言してすぐ取り下げる」だけで、相手を部屋から追い出せる（何度でも繰り返せる）。
  > 改善: liveliness による解放は「その `sub` が最後に検証済みの input を送ってから一定時間経った場合」に限るか、liveliness のキーに Room Token の `jti` のハッシュを含め、host は検証済みの jti としか照合しないようにする。
  > 対象ファイル: `client/host-server/src/main.rs`（L424-457）, `protocol/src/lib.rs`（L116-138）

- **Room Token を共有バスに毎フレーム載せるため、盗聴すれば 300 秒間なりすませる** `-3`
  > `InputEvent.room_token` は Bearer の JWT で、30Hz で `…/input` に publish される。Zenoh には ACL が無く（WAN では暗号化も無く）、誰でも `alchemy/**` を subscribe できるので、他人のトークンを拾って、その人として移動・Grab・Leave を送れる。host は署名を検証するが、送信者がトークンの持ち主かどうかは確かめていない。
  > 改善: 最初の検証済み input で host がセッション鍵を発行し（client の公開鍵で暗号化するか、Room Token に `cnf` / jwk thumbprint を持たせる DPoP 風の方式）、以降は HMAC 付きの input だけを受け付ける。短期的には Zenoh の ACL で `…/input` を subscribe できるのを host だけにする。
  > 対象ファイル: `protocol/proto/input_events.proto`（L9-10）, `client/host-server/src/main.rs`（L349-370）

- **`host_control` の shutdown が無認証** `-2`
  > 部屋のキーに `shutdown` という 8 byte を put するだけで、その部屋の host プロセスが `exit(0)` する。WAN の zenohd に接続できる人なら誰でも host を止められる。「スタブ」と明記されてはいるが、x001b で公網に出した時点でリスクが現実になった。
  > 改善: 同じマシン上の launcher からしか呼ばない前提なら、Zenoh ではなくローカル IPC（名前付きパイプや Unix ソケット）に移す。Zenoh に残すなら、host 自身の Room Token の jti に紐づく署名を要求する。
  > 対象ファイル: `client/host-server/src/host_control.rs`（L10-32）

- **既定の認証情報がバイナリに埋め込まれている** `-1`
  > host-server は `AUTH_USER` / `AUTH_PASSWORD` が未設定だと `alice` / `Password1` で黙ってログインする。client も `AUTH_PASSWORD` の既定値が `Password1`。開発の便宜のためだが、配布バイナリに残すべきではない。
  > 改善: `#[cfg(debug_assertions)]` か feature フラグの下に移す。release ビルドでは未設定ならエラーにする。
  > 対象ファイル: `client/host-server/src/room_token.rs`（L15-17, L170-173）, `client/app/src/auth.rs`（L175）

- **依存の脆弱性管理が CI に無く、hackney の既知の脆弱性を検出したまま** `-1`
  > 当日の assets の `mix deps.get` で、hackney（ex_aws 経由）の CRLF インジェクションに関する advisory（EEF-CVE-2026-47069 / 47075）が表示された。`cargo audit` / `cargo deny` / `mix deps.audit` / `sobelow` は、どの workflow にも無い。
  > 改善: 各 workflow に `cargo deny check advisories` と `mix hex.audit` / `mix deps.audit` を追加し、ex_aws の HTTP クライアントを Req に差し替える（`ex_aws` の `http_client` 設定）。
  > 対象ファイル: `.github/workflows/*.yml`, `assets-server/mix.exs`

### プロジェクト全体設計

- **コンテンツの定義が host のコードに直書きされ、client の描画側にも複製されている（定義と実行の逆転）** `-3`
  > Coin の BLAKE3、箱 9 個の配置と半径、spawn 座標、entity ID の範囲（Coin=1、static=2、user_root=3..6、box=10..18）が、host-server の `sim.rs` と `main.rs` に定数として埋め込まれている。client の `scene.rs` にも「Matches host」というコメント付きで同じ定数（`BOX_ENTITY_FIRST`、`BOX_COUNT`、`BOX_HALF_EXTENT`、`USER_ROOT_SLOTS`）がある。実装ルールの層間インターフェース原則にある「固有の概念を扱っていないか」と「定義 vs 実行の分離」の両方に違反している。World の定義（何が・どこに・どんな形で）は Manifest として上位（world-server）から届き、host はそれを実行し、client は Manifest に従って描画するべき。今のままでは、箱を 1 個増やすだけで 3 つのファイルを同時に直す必要がある。
  > 改善: `SceneEntry` に `shape`（cuboid / ball / mesh）、`half_extents`、`body_type`、`grabbable` を追加し、host は Manifest から Rapier のワールドを構築する。client は Manifest の shape から描画する。entity ID の範囲の取り決めはやめ、Manifest と Presence で割り当てる。
  > 対象ファイル: `client/host-server/src/sim.rs`（L13-39）, `client/host-server/src/main.rs`（L50-53, L308-324）, `client/app/src/scene.rs`（L24-36）

- **protoc によるスキーマ検証が 4 ファイル中 2 ファイルしか対象にしていない** `-1`
  > `protocol-ci.yml` と `ci.sh` の protoc ステップが、`render_frame.proto` と `input_events.proto` しか渡していない。`scene_manifest.proto` と `presence.proto` は CI のスキーマ検証から漏れている。手元では protoc が PATH に無いため、このステップは SKIP になった。
  > 改善: `protocol/proto/*.proto` を glob で渡す。`buf lint` と `buf breaking` を導入すると、PIN を上げる判断も自動化できる。
  > 対象ファイル: `.github/workflows/protocol-ci.yml`（L30-37）, `.scripts/ci.sh`（L40-46）

- **vision.md と launcher.md が空で、一部の文書がコードとずれている** `-1`
  > `.workspace/vision.md` は見出しだけ、`architecture/launcher.md` も見出しだけ。`architecture/zenoh-router.md` の「MVP キー式」表には frame と input しか無く、manifest / presence / alive / host_control が抜けている。roadmap では x001d が `active` のままだが、PR #40 でマージ済み。評価観点の「ビジョンとコードの一致度」は、比較対象が無いので評価できない。
  > 改善: vision に「誰のための何か・やらないこと」を 1 ページで書く。キー表は protocol.md を正本として参照させる。マージ時にスライスの状態を done にするのを PR テンプレートのチェック項目にする。
  > 対象ファイル: `.workspace/vision.md`, `.workspace/0_doc/architecture/launcher.md`, `.workspace/0_doc/architecture/zenoh-router.md`（L25-31）, `.workspace/roadmap/README.md`（L81）
