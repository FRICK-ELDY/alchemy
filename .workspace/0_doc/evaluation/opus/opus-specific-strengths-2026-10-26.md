# Alchemy — プラス点詳細（第1評価者 Claude Opus 5.5 / 2026-10-26）

> 評価対象コミット: `683e30f`（main）。全項目は当日コードを直接読んで確認した。
> 第2評価者の文書は参照していない。

## 採点基準

| 点数 | 基準 |
|:---:|:---|
| +1 | 正しく実装されている。問題はないが特筆するほどではない |
| +2 | 業界の一般的なベストプラクティスに沿った、良い設計判断 |
| +3 | 同規模・同種プロジェクトの平均を明確に上回る実装 |
| +4 | プロダクションレベルのゲームエンジン・OSSと比較しても遜色ない実装 |
| +5 | このクラスの個人プロジェクトでは見たことがないレベルの卓越した実装 |

---

## 技術評価層 — 運営側コア

### auth-server（アイデンティティ SSoT）

- **Refresh Token のファミリー回転と再利用検知** `+4`
  > 平文は 32 byte 乱数を返すだけで、DB には SHA-256 ハッシュのみを保存する。refresh のたびに提示トークンを失効させて同一 `family_id` で再発行し、猶予秒数を過ぎた失効済みトークンが再提示されたらファミリー全体を一括失効させる。非アクティブ期間によるスライディング失効もある。OAuth 2.0 Security BCP の「refresh token rotation + reuse detection」をそのまま実装しており、Auth0 / Keycloak 相当の水準。
  > ```elixir
  > {:ok, %{revoked_at: revoked_at} = record} when not is_nil(revoked_at) ->
  >   grace_seconds = Application.fetch_env!(:auth, :refresh_token_reuse_grace_seconds)
  >   if DateTime.diff(now, revoked_at, :second) > grace_seconds do
  >     revoke_refresh_token_family(record.family_id, now)
  > ```
  > 対象ファイル: `auth-server/lib/auth/accounts.ex`（L104-134, L307-358, L398-402）

- **RS256 + `kid`（JWK thumbprint）+ ローテーション用検証鍵** `+3`
  > 署名鍵 1 本と検証専用鍵（`JWT_VERIFICATION_KEY_PATHS`）を分け、全鍵を JWKS に載せることで鍵ローテーションの猶予期間を作れる。`kid` 重複は起動時に例外で止める。prod では鍵の自動生成を `false` に固定しており、鍵が無ければ起動しない。複数鍵の JWKS テスト（`jwks_controller_multi_key_test.exs`）もある。
  > 対象ファイル: `auth-server/lib/auth/token/keys.ex`（L54-83, L102-113）, `auth-server/config/runtime.exs`（L94-123）

- **多軸レート制限（IP × 識別子 × トークンファミリー）** `+3`
  > ETS の `update_counter` による固定窓のロックフリー計数。login は IP と identifier、refresh は IP と refresh token ファミリーで二重に制限する。初回超過時だけ telemetry `[:auth, :rate_limit, :throttle]` を発火し、`retry-after` も返す。閾値はすべて環境変数で調整でき、古い窓は定期的に掃除される。
  > 対象ファイル: `auth-server/lib/auth/rate_limit.ex`（L73-107, L129-148）, `auth-server/lib/auth_web/plugs/rate_limit.ex`（L45-78）

- **アカウント列挙・タイミング攻撃への配慮** `+2`
  > ログイン失敗メッセージは「ユーザー名かメールまたはパスワードが違う」に統一し、ユーザーが存在しない場合も `Argon2.no_user_verify/0` でハッシュ計算時間を揃える。forgot-password / resend-verification はアカウントの有無に関係なく同じ応答を返す。登録時の一意制約違反も汎用メッセージに丸める。
  > 対象ファイル: `auth-server/lib/auth/accounts.ex`（L13-16, L56-78, L179-201）, `auth-server/lib/auth/password.ex`

- **メール確認・パスワードリセットのトランザクション設計** `+2`
  > アカウントトークンは `FOR UPDATE` で行ロックを取ってから消費する。リセット時は同一トランザクション内でパスワード変更・トークン消費・refresh 全失効を行う。新規発行時は同じ目的の未使用トークンを無効化する。
  > 対象ファイル: `auth-server/lib/auth/accounts.ex`（L203-227, L429-466）

- **認証 Plug の失敗分類と、応答に情報を漏らさない構造化ログ** `+2`
  > 失敗理由を `missing_token / expired_token / revoked_token / forbidden …` に分類してログには詳細を残し、クライアントには 401/403 のテンプレート応答だけを返す。予期しない例外も rescue して 401 に落とす。
  > 対象ファイル: `auth-server/lib/auth_web/plugs/authenticate.ex`（L41-67, L151-168）

- **公開 / レート制限 / 認証必須のパイプライン分離** `+1`
  > `/register` などの公開 API は `:rate_limited`、`/logout` `/me` などは `:authenticated`、`/health` と `/.well-known/jwks.json` はパイプラインなしで公開する。境界が明確。
  > 対象ファイル: `auth-server/lib/auth_web/router.ex`（L4-40）

### assets-server（CAS・Quota・門番）

- **メタと不変バイナリの分離 + BLAKE3 CAS の重複排除** `+2`
  > DB（SQLite）は `blake3 / size / mime / owner_sub` と Quota だけを持ち、本体は CAS に置く。同じハッシュの再アップロードは `:existing` を返し、CAS への書き込みも Quota への加算もしない。architecture の「保証する」表どおり。
  > 対象ファイル: `assets-server/lib/assets/blobs.ex`（L32-73）

- **CAS バックエンドの behaviour 抽象と、起動時の必須 env 検査** `+2`
  > `Assets.CAS.Backend` behaviour の裏に `Local` と `S3` を置き、`ASSETS_CAS_BACKEND=local|s3` で切り替える。s3 のときは bucket / key / endpoint が欠けていれば起動時に明示的なメッセージで落ちる。endpoint からホストを取り出す処理には、ExAws が R2 で URL を壊す既知の罠への対処コメントもある。
  > 対象ファイル: `assets-server/config/runtime.exs`（L14-68）, `assets-server/lib/assets/cas/s3.ex`（L102-114）

- **s3 時の GET を、ローカル署名した presigned URL への 302 にする** `+2`
  > ネットワークアクセスなしで署名 URL を生成し、client は R2 から直接ダウンロードする（egress がサーバーを通らない）。TTL は env で変更できる。overview の⑦経路の「ホスト負荷 0」を最小コストで実現している。
  > 対象ファイル: `assets-server/lib/assets/cas/s3.ex`（L51-61）, `assets-server/lib/assets_web/controllers/blob_controller.ex`（L33-36）

- **local CAS の原子的書き込み（tmp → rename）** `+1`
  > 一時ファイルへ書いてから rename するので、途中で落ちても壊れたファイルが CAS キーに残らない。
  > 対象ファイル: `assets-server/lib/assets/cas/local.ex`（L14-32）

### world-server（案内所・門番）

- **Room Token の claim 設計** `+2`
  > `sub / room_id / role / iss=alchemy-world / aud=alchemy-room / exp(300s) / jti` を持つ RS256 トークン。`kid` は thumbprint で、Room Token 専用の JWKS を公開する。auth の JWT とは iss / aud が分かれており、取り違えて使えない。
  > 対象ファイル: `world-server/lib/world/token.ex`（L10-60）, `world-server/lib/world/token/keys.ex`

- **auth JWT 検証と `sub` の転写（body の `sub` は無視）** `+2`
  > iss / aud / status を検証し、Room Token の `sub` は Bearer JWT から取る。リクエスト body に `sub` を入れても無視されることがテストで固定されている（`"ignored-body-sub"`）。
  > 対象ファイル: `world-server/lib/world/auth/verify.ex`（L22-51）, `world-server/lib/world_web/controllers/room_controller.ex`（L42-54）, `world-server/test/world_web/controllers/room_controller_test.exs`（L16-33）

- **匿名トークンを既定で OFF にし、env で両方向を明示** `+1`
  > `WORLD_ALLOW_ANON_TOKENS` は true/false の両方を明示的に解釈する。docker-compose でも `"false"` を明記している。
  > 対象ファイル: `world-server/config/runtime.exs`（L19-26）, `world-server/docker-compose.yml`（L18）

- **room_id の厳格な検証** `+1`
  > `^[A-Za-z0-9_-]+$`、最大 64 byte、`/` 禁止。
  > 対象ファイル: `world-server/lib/world/rooms.ex`（L75-92）

---

## 技術評価層 — 共通契約層

### protocol/（ワイヤ契約 SSoT）

- **チャネル責務の分離（Manifest / frame / presence / input / liveliness）** `+3`
  > 低頻度の「何が置かれているか」（SceneManifest は BLAKE3 参照のみで GLB を載せない）、高頻度の「どこにあるか」（RenderFrame は upserts/removes のデルタ）、Presence（`sub` → entity）、Intent（InputEvent）を別メッセージ・別キーに分けている。Resonite や VRChat 系の「アセット参照と姿勢ストリームの分離」と同じ発想で、ルーター負荷を抑える設計として正しい。
  > 対象ファイル: `protocol/proto/scene_manifest.proto`, `protocol/proto/render_frame.proto`, `protocol/proto/presence.proto`, `protocol/proto/input_events.proto`

- **client 外の独立クレート + protoc 同梱ビルド** `+2`
  > `protocol/` はスーパープロジェクト直下に置き、client workspace からは `path = "../protocol"` で参照する（埋め込まない方針と一致）。`protoc-bin-vendored` を使うので、`protoc` が無い環境でもビルドが通る。
  > 対象ファイル: `protocol/build.rs`, `client/Cargo.toml`（L14-15）

- **キー式ヘルパ + `validate_room_id` + liveliness キーの解析とテスト** `+2`
  > host と client が同じ関数でキーを組み立てるので、文字列リテラルの食い違いが起きない。`liveliness_sub_from_key` は余分なセグメントを持つキーを拒否する。単体テストは 7 件。
  > 対象ファイル: `protocol/src/lib.rs`（L74-138, L277-316）

---

## 技術評価層 — ホストユーザー端末

### client/host-server（Untrusted 計算ホスト）

- **Presence スロット解放の三重化（明示 Leave / liveliness Delete / Intent TTL）** `+3`
  > 明示退室はトークンを検証したうえで即時解放し、プロセスの突然死は liveliness の Delete で、ネットワーク断は 3 秒の Intent 無音で回収する。入力スレッドで解放した entity は `pending_removes` に積み、物理スレッド側で `release_slot` と removes 配信に反映する。テストもある。
  > 対象ファイル: `client/host-server/src/main.rs`（L106-180, L424-457, L600-629, L772-833）

- **内部物理 60Hz と配信 30Hz の分離 + 固定ステップ** `+2`
  > `next_physics += period` の固定ステップで回し、`PHYSICS_PER_NET` 回に 1 回だけデルタを配信する。間のステップで解放が起きたときも removes だけは即座に流す。
  > 対象ファイル: `client/host-server/src/main.rs`（L44-47, L567-585, L668-684）

- **入力ゲートの順序と、拒否理由ごとのカウンタ** `+2`
  > decode → Room Token 検証 → Leave 判定 → `is_finite` → スロット割当の順で処理し、`dropped_token / dropped_non_finite / dropped_full` を 5 秒ごとにログへ出す。Untrusted 入力を物理より前で落とす構造が明確。
  > 対象ファイル: `client/host-server/src/main.rs`（L349-417）, `client/host-server/src/finite.rs`

- **Room Token 検証の厳格さ（alg 固定・iss/aud/exp・room_id 束縛）** `+2`
  > `Validation::new(Algorithm::RS256)` で alg を固定し（`alg=none` や HS256 への混同攻撃を防ぐ）、トークンの `room_id` が自分の部屋と一致しなければ拒否する。
  > 対象ファイル: `client/host-server/src/room_token.rs`（L112-133）

- **Rapier ワールドの SoA 構成と、Grab の no-steal** `+2`
  > grabbable box の handle / entity_id / grabbable フラグは SoA の固定長配列。他スロットが持っている箱は掴めない。落下・着地・追従・解放・spawn 復帰はユニットテストで固定されている（11 件）。
  > 対象ファイル: `client/host-server/src/sim.rs`（L59-103, L229-302, L437-628）

### client/app（体感・描画・UI エンドポイント）

- **system_ui が OpenXR / winit / Room を知らない** `+3`
  > `on_pointer(px, py, primary_down)` だけを受け取り、デバイス差は surface 側で吸収する。DashRoot は Room の frame に載せないことをモジュール契約として明記している。層間インターフェース原則の「固有知識を持ちすぎない」を実際のコードで守れている数少ない層。
  > 対象ファイル: `client/app/src/system_ui/mod.rs`（L1-9, L80-99）, `client/app/src/system_ui/facet.rs`（L1-24）

- **OpenXR の実機コントローラ描画（render model・animatable node・baseColor）** `+3`
  > `XR_EXT_render_model` からランタイムの GLB を取り、ルート pose と animatable node（トリガー・スティック）を反映する。スナップ／スムーズのコンフォートターン、ダッシュ用レーザーもある。個人プロジェクトで OpenXR をここまで実機に寄せた例は少ない。
  > 対象ファイル: `client/app/src/surface/xr/render_model.rs`, `client/app/src/turn.rs`, `client/app/src/surface/xr/input.rs`

- **surface の自動選択とフォールバック** `+2`
  > `ALCHEMY_FORCE_DESKTOP` → Desktop、OpenXR の probe 失敗 → Desktop、XR 実行中の失敗 → Desktop へ降格する。
  > 対象ファイル: `client/app/src/surface/mod.rs`（L22-59）

- **デコード前の BLAKE3 照合と 302 追従** `+2`
  > assets から取った bytes は `verify_blake3` を通してから GLB デコーダへ渡す（毒データの排除）。302 を最大 5 回まで追従し、別ホストへのリダイレクトでは ureq が Authorization を外すことをコメントで明記している。
  > 対象ファイル: `client/app/src/main.rs`（L106-122, L574-588）

- **Leave → liveliness undeclare の順序設計** `+1`
  > 明示 Leave を publish した後に liveliness トークンを drop し、その後でローカル状態を消す。host 側で解放の経路が二重に効く。
  > 対象ファイル: `client/app/src/main.rs`（L437-471）

- **サブルーター化しない純粋エンドポイント** `+1`
  > client が publish するのは自分の input（と host_control スタブ）だけで、他人のデータを中継しない。
  > 対象ファイル: `client/app/src/main.rs`（L159-396）

---

## 技術評価層 — 転送・公開エッジ

### zenoh-router / edge

- **ローカル / WAN 設定の分離と差分の明記** `+2`
  > WAN 用は multicast scouting を OFF にし、その差分をファイル先頭のコメントに書いている。README の比較表とも一致する。
  > 対象ファイル: `zenoh-router/config.wan.json5`（L1-4）, `zenoh-router/README.md`（L17-25）

- **Cloudflare Tunnel テンプレート（認証情報は gitignore、catch-all 404）** `+2`
  > `credentials.json` と実 `config.yml` はコミットせず、ingress の最後は `http_status:404` で閉じる。ホスト名ごとに振り分け、ルーターのポートは開けない。
  > 対象ファイル: `edge/cloudflared/config.yml.example`, `edge/cloudflared/.gitignore`

- **WAN の運用手順（systemd・ufw allowlist）** `+1`
  > 7447 番ポートを全世界に開けないことを手順書の前提にしている。
  > 対象ファイル: `.workspace/0_doc/manual/zenoh-wan-conoha.md`

- **「zenohd はトークンを見ない」という境界の明記** `+1`
  > 検証は host（と将来の Relay Agent）の責務であることを、config・README・architecture の 3 か所で一貫して書いている。
  > 対象ファイル: `zenoh-router/README.md`（L69-71）, `.workspace/0_doc/architecture/zenoh-router.md`

---

## 横断評価層

### テスト戦略

- **auth-server の厚いテスト（107 件）** `+3`
  > ライフサイクル、複数鍵の JWKS、レート制限（Plug と ETS の両方）、トークンの掃除、エラー JSON まで揃っている。当日 Docker 上の `mix test` で 107 件すべて成功した（format / compile --warnings-as-errors / credo --strict も通過）。
  > 対象ファイル: `auth-server/test/**`

- **Rust のユニットテスト（client 55 / host 19 / protocol 7）** `+2`
  > 物理・Presence・finite・system_ui・turn・world_hit・protocol の往復をカバーしている。当日 WSL で `cargo fmt --check / clippy -D warnings / build / test` がすべて通過した。
  > 対象ファイル: `client/**`, `protocol/src/lib.rs`

### 可観測性

- **auth の構造化失敗ログと telemetry** `+2`
  > `event: "auth.authenticate.failure"` と `auth.rate_limit.throttle` を map 形式で出力し、失敗の種類で warning と error を使い分けている。
  > 対象ファイル: `auth-server/lib/auth_web/plugs/authenticate.ex`（L155-168）, `auth-server/lib/auth/rate_limit.ex`（L90-107）

- **host の周期カウンタ** `+1`
  > 入力ゲート（accepted / leaves / dropped_*）と物理（steps・delta 件数・空フレーム数）を 1 秒または 5 秒ごとにまとめてログへ出す。
  > 対象ファイル: `client/host-server/src/main.rs`（L404-417, L739-768）

### エラーハンドリング

- **Rust は anyhow + context、Elixir はタグ付きタプル + FallbackController** `+2`
  > 起動時の失敗には「is world-server up?」のような行動を促す文脈が付いている。Elixir は `{:ok, _} / {:error, _}` で統一されている。
  > 対象ファイル: `client/host-server/src/main.rs`（L486-492）, `world-server/lib/world_web/controllers/fallback_controller.ex`

### 変更容易性・保守性

- **保証単位のファイル分割と、モジュールごとの Guarantees / Non-guarantees** `+3`
  > Rust の `//!` と Elixir の `@moduledoc` のほぼすべてに「保証する／しない」がある（例: `Assets.Blake3`「Non-guarantees: streaming hash API」、`finite.rs`「Non-guarantees: magnitude clamps」）。実装ルールの「ファイル単位で保証を明確にする」をコードベース全体で実践している。TODO / FIXME はコード内に 0 件。
  > 対象ファイル: `assets-server/lib/assets/blake3.ex`（L1-7）, `client/host-server/src/finite.rs`（L1-4）ほか

### 開発者体験（DX）

- **README → WSL セットアップ → compose → cargo run、`make ci` を単一入口に** `+3`
  > `wsl-setup.sh` で依存を入れ、`make ci` が GitHub Actions と同じゲートを手元で再現する。Windows ネイティブで cargo が動かない理由（os error 4551）まで明記している。
  > 対象ファイル: `README.md`, `Makefile`, `.scripts/ci.sh`, `.scripts/wsl-setup.sh`

- **Elixir 系は全 OS で Docker、auth には prod release ステージもある** `+1`
  > 対象ファイル: `auth-server/Dockerfile`（L31-71）

### セキュリティ・配布可能性

- **Untrusted host 前提の入口防御（入力ごとの RS256 検証 + 物理前の `is_finite`）** `+2`
  > 対象ファイル: `client/host-server/src/main.rs`（L352-369）, `client/host-server/src/finite.rs`

- **秘密情報の取り扱い** `+1`
  > `.env` や tunnel の認証情報は gitignore 済み。prod では鍵の自動生成を拒否する。
  > 対象ファイル: `.gitignore`, `auth-server/config/runtime.exs`（L123）

### プロジェクト全体設計

- **architecture/*.md の「保証する／しない」表がコードと一致している** `+4`
  > protocol.md の `PROTOCOL_PIN`（`0.9.0-x0019`）、client.md のチャネルキー一覧、world-server.md の claim 表、assets-server.md の API 表が、いずれも当日コードと一致した。目標設計（overview）と現状（各 md）を明確に分け、目標を現状のように書かない規律は、同規模の OSS でもまれ。
  > 対象ファイル: `.workspace/0_doc/architecture/*.md`

- **縦スライスのロードマップ規律** `+3`
  > `x` + 16 進 4 桁の不変 ID、1 Slice = 1 ファイル、ブランチ名との対応、depends_on。29 本のスライスが「保証する／しない」付きで積み上がっている。
  > 対象ファイル: `.workspace/roadmap/README.md`, `.workspace/roadmap/slices/`

- **旧 Elixir 権威実行からの転換（Pure Elixir world + Zenoh 一本化）** `+2`
  > world-server に NIF・独自 UDP・空間用 Channel は無く、空間計算は Pure Rust の host に移っている。overview の方針とコードが一致する。
  > 対象ファイル: `world-server/mix.exs`, `world-server/lib/**`

- **コンポーネント単位でパスフィルタした CI** `+1`
  > 対象ファイル: `.github/workflows/*.yml`
