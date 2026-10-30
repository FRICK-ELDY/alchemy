# Alchemy — マイナス点詳細（第1評価者 Claude Opus 5.5 / 2026-10-28）

> 評価対象コミット: `6a0a879`。全項目は当日コードを直接読んで確認した。
> 前回（`archive/2026-10-26/`）からの継続項目には「継続」、今回新たに見つけたものには「新規」と付記する。

## 採点基準

| 点数 | 基準 |
|:---:|:---|
| -1 | 軽微な問題。改善が望ましいが現状でも機能する |
| -2 | 明確な問題。放置すると将来的に技術的負債になる |
| -3 | 深刻な問題。現時点でバグ・パフォーマンス劣化・保守困難を引き起こしている |
| -4 | 致命的な問題。設計の根本的な見直しが必要 |
| -5 | プロジェクトの存続に関わる問題 |

| ブロック | 観点 | 小計 |
|:---|:---|---:|
| 運営側コア | auth-server | -2 |
| 運営側コア | assets-server | -2 |
| 運営側コア | world-server | -2 |
| 共通契約 | protocol/ | -1 |
| ホスト端末 | client/host-server | -4 |
| ホスト端末 | client/app | -4 |
| ホスト端末 | launcher/ | -1 |
| 転送・エッジ | zenoh-router / edge | -4 |
| 横断 | テスト戦略 | -4 |
| 横断 | 可観測性 | -2 |
| 横断 | エラーハンドリング | -1 |
| 横断 | 変更容易性・保守性 | -1 |
| 横断 | 開発者体験（DX） | -1 |
| 横断 | セキュリティ・配布可能性 | -5 |
| 横断 | プロジェクト全体設計 | -2 |
| | **合計** | **-36** |

---

## 技術評価層 — 運営側コア

### auth-server

- **`Auth.Token.Keys` が全 JWT 署名を 1 つの GenServer で直列化する（継続）** `-1`
  > `signer/0` と `jwks/0` が毎回 `GenServer.call` を通る。鍵は起動後に変わらないので `persistent_term` に置けば直列化もメッセージコピーも無くなる。現在の規模では問題にならないが、login と refresh のスループットの上限がここで決まる。
  > 対象ファイル: `auth-server/lib/auth/token/keys.ex`

- **`/internal/*` が Tunnel の ingress で塞がれておらず、Social / internal にレート制限が無い（新規）** `-1`
  > `/internal/v1/access` は共有トークンで守られているが、ingress は `auth.example.com` の全パスを auth に流すため、内部 API が公網から到達できる（`/dev/*` は 404 にしているのに `/internal/*` は無い）。トークンの総当たりは現実的ではないが、多重防御の方針と一貫しない。また Follow / Friendship の API は `:authenticated` だけで `:rate_limited` を通らず、1 アカウントで大量のフォローや申請を作れる。
  > 対象ファイル: `edge/cloudflared/config.yml.example`（L15-20）, `auth-server/lib/auth_web/router.ex`（L40-63）

### assets-server

- **BLAKE3 を外部 CLI `b3sum` と一時ファイルで計算する（継続）** `-1`
  > アップロードごとに一時ファイルの書き出しと外部プロセスの起動が発生する。Rust NIF か、Elixir 側でのストリーミング計算に置き換えたい。
  > 対象ファイル: `assets-server/lib/assets/blake3.ex`

- **CAS への put（R2 へのアップロード）を SQLite のトランザクション内で行う（新規）** `-1`
  > `Blobs.store/3` は `Repo.transaction` の中で Quota の消費と `CAS.put` を行う。s3 バックエンドでは HTTP アップロードの間ずっと SQLite の書き込みロックを持つため、1 件の遅いアップロードが他のすべての書き込みを止める。また、put 成功後にメタの insert が失敗すると、CAS に孤児オブジェクトが残る（CAS は内容アドレスなので害は小さい）。「Quota を予約 → トランザクション外で put → メタを確定」の 2 段階にするべき。
  > 対象ファイル: `assets-server/lib/assets/blobs.ex`（L36-56）

### world-server

- **Room を作れる数に上限が無い（継続）** `-1`
  > 認証済みユーザーなら誰でも `POST /rooms` で部屋を作り続けられる。永続化されたので、前回よりも影響が大きい（SQLite が際限なく増える）。ユーザーごとの上限か、作成のレート制限が必要。
  > 対象ファイル: `world-server/lib/world_web/controllers/room_controller.ex`, `world-server/lib/world/rooms.ex`

- **共有 JWKS キャッシュが取得失敗時にクールダウンせず、GenServer 内で同期取得する（部分的に解決）** `-1`
  > 前回の「コピー」は解消した。成功後の未知 kid は 1 秒のクールダウンが付いたが、auth が落ちているときは失敗ごとに再取得を試み、しかも GenServer の中で同期 HTTP を行うので、待っている全リクエストが 1 本の HTTP タイムアウトの後ろに並ぶ。auth の障害が world と assets の全リクエストの遅延に増幅される。
  > 対象ファイル: `shared/alchemy_auth_client/lib/alchemy_auth_client/jwks.ex`（L148-173）

---

## 技術評価層 — 共通契約層

### protocol/

- **user_root の ID 帯と `USER_ROOT_SLOTS=4` を protocol の定数として固定（新規）** `-1`
  > 参加者数の上限という「部屋の定義」が、汎用であるべきワイヤ契約クレートの定数になっている。部屋ごとに上限を変えるには protocol の更新と全バイナリの再ビルドが要る。上限は world の Room 定義（Manifest）で配り、protocol は ID の帯の規則だけを持つのが層の原則に合う。
  > 対象ファイル: `protocol/src/lib.rs`（USER_ROOT 関連の定数）

---

## 技術評価層 — ホストユーザー端末

### client/host-server

- **資格情報の更新を物理ループ内で同期 HTTP 実行し、1 回失敗するとプロセスごと終了する（新規）** `-2`
  > 60Hz の物理ループが毎 tick `creds.refresh_if_due` を呼び、期限が来ていれば auth → host Room Token → attestation、続けて seal を ureq で同期取得する。タイムアウトは各 5 秒で、コメント自身が「直列に積み重なる」と書いている。最悪で約 20 秒、物理と配信が止まる。さらに、1 回でも失敗すると `return Err` でプロセスが終了するので、auth の一時的な 502 で部屋全体が消える（launcher があれば再起動するが、全員の状態は失われる）。期限の手前で非同期にリトライし、本当に失効するまで現在の資格情報で動き続けるべき。
  > 対象ファイル: `client/host-server/src/main.rs`（L851-889）, `client/host-server/src/room_token.rs`（L7-9）

- **Grab 保持中の `aim` に上限が無い（新規）** `-1`
  > `move` はクランプしたが、Grab を保持している間は `aim` がそのまま保持オフセットになり、箱を任意の遠方へ瞬間移動させられる。`finite.rs` は非保証として正直に書いているが、overview の「100% 遮断」という表現とは一致しない。
  > 対象ファイル: `client/host-server/src/sim.rs`（`follow_grab` 周辺）, `client/host-server/src/finite.rs`（L1-5）

- **`main()` が約 450 行の一枚岩で、publish の失敗でも `?` で終了する（継続）** `-1`
  > 起動・購読・物理ループ・配信・資格情報更新・周期ログが 1 関数に入っている。ループ内の `put()?` も、Zenoh の一時的な失敗でプロセスを止める。
  > 対象ファイル: `client/host-server/src/main.rs`（L640-1100 付近）

### client/app

- **描画経路が 2 本のまま。`vk_loop.rs` は 2720 行に増えた（継続）** `-2`
  > desktop の wgpu と XR の生 Vulkan が別実装で、同じ機能（TRS・コントローラ・UI）を 2 回書いている。前回より行数が増え、二重実装のコストは上がり続けている。
  > 対象ファイル: `client/app/src/surface/xr/vk_loop.rs`, `client/app/src/surface/desktop.rs`

- **client の `world_jwks.rs` が未知 kid ごとに同期で再取得し、クールダウンが無い（新規）** `-1`
  > host 側は前回の指摘を受けてクールダウン付きの SWR にしたが、client 側の world JWKS は未知 kid を見るたびに同期 HTTP を行う。偽 Manifest（kid を毎回変える）を流されると、Zenoh 受信スレッドが world への HTTP で詰まり、world にも負荷がかかる。
  > 対象ファイル: `client/app/src/world_jwks.rs`（L92-106）

- **register が `"tos_agreed": true` を固定で送る（継続）** `-1`
  > 利用規約への同意をユーザーが操作していないのに、同意したことになる。
  > 対象ファイル: `client/app/src/auth.rs`

### launcher/

- **launcher が CI の対象外で、clippy が失敗し、テストが 0 件（新規）** `-1`
  > `cargo clippy -- -D warnings` が `needless_question_mark` で 2 件失敗する（当日確認）。`launcher/` は client workspace の外にあり、どの workflow も触れていない。再起動回数・バックオフ・ready ファイル待ちはテストしやすい純粋関数に切り出せるのにテストが無い。
  > 対象ファイル: `launcher/src/main.rs`（L194, L208）, `.github/workflows/`

---

## 技術評価層 — 転送・公開エッジ

### zenoh-router / edge

- **prod compose が `AUTH_INTERNAL_TOKEN`（と world の `AUTH_INTERNAL_URL`）を渡さず、手順書どおりでは起動しない（新規）** `-3`
  > auth と world の `runtime.exs` は prod で `AUTH_INTERNAL_TOKEN` が無いと raise するが、`auth-server/docker-compose.prod.yml` と `world-server/docker-compose.prod.yml` の `environment:` にこの変数が無く、`.env.prod.example` にも記載が無い。手順書は `--env-file` で起動するよう指示するが、`--env-file` は compose ファイル内の変数置換にしか使われず、コンテナには渡らない。結果として、手順書どおりに prod を起動すると両サービスが起動時に落ちる。さらに world の S2S 既定 URL は `http://127.0.0.1:4002` で、コンテナ内では自分自身を指す。x0035（Social ACL）で追加した必須設定が、x002f で整えた prod 経路に反映されていない。前回 `-4` で直した「公開手順」が、別の形で再び壊れている。
  > 対象ファイル: `auth-server/docker-compose.prod.yml`, `world-server/docker-compose.prod.yml`, `auth-server/.env.prod.example`, `world-server/.env.prod.example`, `auth-server/config/runtime.exs`（L78-86）, `world-server/config/runtime.exs`

- **Zenoh に ACL もクライアント認証も無い（TLS のみ）（継続・一部解決）** `-1`
  > 盗聴は TLS で防げるようになったが、zenohd に到達できる誰でも任意のキーに publish できる。署名のおかげで Frame / Presence / Intent の偽造は破棄されるが、下記のセキュリティの重大弱点のように「破棄」ではなく「退室」する経路があると、ACL が無いことが直接被害になる。`access_control` でキー族ごとの publish 権を絞るか、mTLS で接続元を限定したい。
  > 対象ファイル: `zenoh-router/config.wan.json5`

---

## 横断評価層

### テスト戦略

- **world のテストが開発用 DB に向き、`make ci` が手元で落ちる（新規）** `-2`
  > `world-server/config/runtime.exs`（L60 付近）は `WORLD_DATABASE_PATH` を環境を問わず適用し、日常の compose は `/app/priv/data/world_dev.db` を設定している。そのため、コンテナ内で `mix test` を実行すると、日常スタックで作った "local" 部屋が入った dev DB をそのまま使い、`room_controller_test.exs:16` が 409 で失敗する（当日 2 回とも再現）。`WORLD_DATABASE_PATH=/tmp/...` を指定すると 49 件すべて成功したので原因は確定している。前回 assets で直した「開発者の環境変数がテストに漏れる」とまったく同じ型の再発で、assets では隔離テストを書いたのに world には横展開されていない。また `rooms_test` と `room_controller_test` は Repo を再起動した後に Sandbox を `:manual` に戻していない。
  > 対象ファイル: `world-server/config/runtime.exs`, `world-server/docker-compose.yml`, `world-server/test/world/rooms_test.exs`, `.scripts/ci.sh`

- **host の Room Token 検証に否定系テストが無い（継続）** `-1`
  > `verify` の alg・iss・aud・exp・room_id 不一致を拒否することのテストが無い。セッション bind の正常系はあるが、セキュリティ境界の本体の否定系が抜けている。
  > 対象ファイル: `client/host-server/src/room_token.rs`

- **E2E は正常系のみで、古い smoke は前提が無いとスキップして `exit 0` を返す（新規）** `-1`
  > 新しい compose E2E は正常系 1 本だけで、「偽 Manifest」「不正署名 Intent」「失効トークン」などの否定系が無い。古い `smoke-*.sh` は前提サービスが無いと警告して成功扱いで終わるため、CI に組み込むと何も検証せずに緑になる。
  > 対象ファイル: `client/e2e-probe/src/main.rs`, `.scripts/smoke-*.sh`

### 可観測性

- **メトリクス・分散トレース・相関 ID が無い（継続）** `-2`
  > 3 サービスと host にまたがる 1 回の入室を追う手段が無い。Room Token の `jti` という自然な相関キーがあるのにログで使っていない。host のカウンタもログ出力のみで、外部から収集できない。
  > 対象ファイル: `*/lib/**/application.ex`, `client/host-server/src/main.rs`

### エラーハンドリング

- **world と assets の応答に `detail: inspect(reason)` が残っている（継続）** `-1`
  > 内部エラーの Elixir 表現（構造体・ファイルパス・SQL 断片）がクライアントへ返る。ログに残し、応答には分類コードだけを返すべき。
  > 対象ファイル: `world-server/lib/world_web/controllers/room_controller.ex`, `assets-server/lib/assets_web/controllers/blob_controller.ex`

### 変更容易性・保守性

- **`ClientIp` が auth と world でコピーされている（新規）** `-1`
  > 信頼プロキシの判定と `CF-Connecting-IP` の解析が 2 つのリポジトリ内パッケージに別々にある。セキュリティ境界の判定コードなので、片方だけ修正されると判定が食い違う。`alchemy_auth_client` と同じように共有パッケージにしたい。
  > 対象ファイル: `auth-server/lib/auth_web/client_ip.ex`, `world-server/lib/world_web/client_ip.ex`

### 開発者体験（DX）

- **README どおりに `make ci` を実行すると world が失敗する（新規）** `-1`
  > 上記のテスト DB の問題により、日常スタックを一度でも起動した開発者の手元では `make ci` が赤になる。新しい参加者が最初に見る結果が「壊れている」になる。
  > 対象ファイル: `.scripts/ci.sh`, `.scripts/ci-world-local.sh`

### セキュリティ・配布可能性

- **偽の Manifest / Presence を 1 パケット受け取ると、署名検証の前に部屋から退室する（新規・重大）** `-3`
  > client は Manifest の protocol_pin 不一致・room_id 不一致・`host_pub` の長さ不正・seal 検証失敗・attestation 検証失敗のいずれでも `shared.leave_room_local()` を呼ぶ。Presence でも、`host_sig` を検証するより前の pin 不一致で退室する。`leave_room_local` は Room Token・セッション・liveliness・ワールド表示をすべて消し、自動再入室も無い。Zenoh には ACL が無いので、zenohd に到達できる誰でも「pin だけ違う空の Manifest」を 1 つ publish するだけで、部屋の全員を同時に退室させられる。署名や seal は「偽物を受け入れない」ためのもので、「偽物を見たら逃げる」のでは保護が攻撃手段に変わってしまう。Frame は不正署名なら破棄するだけなので、扱いも一貫していない。検証の失敗は「破棄してログ」にし、退室は検証済みの Manifest（正規の host が世代を変えた場合など）に限るべき。
  > 対象ファイル: `client/app/src/main.rs`（L212-257, L604-611）, `client/app/src/scene.rs`（`leave_room_local`）, `.workspace/0_doc/architecture/trust-boundary.md`

- **bind 時の JWT を盗聴されるとセッションを乗っ取られる（後勝ちの再 bind・`cnf` 無し）（新規）** `-2`
  > 定常 Intent から JWT は外れたが、bind では JWT がワイヤに載る。Room Token に `session_pub` を結び付けるもの（RFC 7800 の `cnf` クレーム相当）が無く、同じ `sub` の再 bind は後勝ちなので、bind を盗聴した攻撃者は TTL 内に自分の鍵で再 bind し、正規ユーザーを追い出して操作を乗っ取れる。WAN は TLS になったが、LAN（`tcp/`）と zenohd を共有する他の参加者からは見える。`session.rs` は非保証として明記しており、誠実さは評価するが、危険度は残る。
  > 対象ファイル: `client/host-server/src/session.rs`（L1-7, L88-115）, `world-server/lib/world/token.ex`

### プロジェクト全体設計

- **定義と実行の逆転が一部残る（継続・大部分解決）** `-1`
  > Scene は world が正本になったが、(1) Elixir の `DemoScene` と Rust のテスト用フィクスチャを手で同期している、(2) client が起動時に Coin のメッシュハッシュを決め打ちで読む、(3) 参加者上限が protocol の定数（上記）、の 3 点が残る。
  > 対象ファイル: `world-server/lib/world/demo_scene.ex`, `client/shared-sim/src/lib.rs`, `client/app/src/main.rs`

- **文書の鮮度：`vision.md` が見出しのみ、README とロードマップの現在地が古い（継続）** `-1`
  > `vision.md` は前回から見出しだけのまま。README には「現状 x0020」、ロードマップには「次 = x0037」とあるが、x0037 は完了済み。architecture/*.md はよく同期されているだけに、入口の文書が古いのが目立つ。
  > 対象ファイル: `.workspace/0_doc/vision.md`, `README.md`, `.workspace/roadmap/README.md`
