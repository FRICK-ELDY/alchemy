# Alchemy — マイナス点詳細（第1評価者 Claude Opus 5.5 / 2026-10-30）

> 評価対象コミット: `d2d9f44`（main・PR #105 マージ直後）。前回 `6a0a879`（2026-10-28）から 90 コミット（x0038〜x0053 の実装・文書、x0054〜x0065 は planned 文書のみ）。
> 全項目は当日コードを直接読んで確認した。第2評価者（`gpt/`）の文書は参照していない。
> 前回からの継続項目には「継続」、一部解消は「部分解消」、今回新たに見つけたものには「新規」と付記する。

## 採点基準

| 点数 | 基準 |
|:---:|:---|
| -1 | 改善余地あり。動作はするが設計・品質上の軽微な問題 |
| -2 | 重要な機能・設計の欠如。放置すると将来の拡張を阻害する |
| -3 | 設計上の明確な欠陥。バグ・クラッシュ・性能劣化を引き起こしうる |
| -4 | プロジェクトの価値命題を損なう重大な欠如。説明責任が果たせない |
| -5 | プロジェクトの根幹を揺るがす致命的な欠陥。存在しないに等しい |

| ブロック | 観点 | 小計 |
|:---|:---|---:|
| 運営側コア | auth-server | -1 |
| 運営側コア | assets-server | -2 |
| 運営側コア | world-server | -4 |
| 共通契約 | protocol/ | -1 |
| ホスト端末 | client/host-server | -4 |
| ホスト端末 | client/app | -2 |
| ホスト端末 | launcher/ | -1 |
| 転送・エッジ | zenoh-router / edge / relay-agent | -7 |
| 横断 | テスト戦略 | -4 |
| 横断 | 可観測性 | -2 |
| 横断 | エラーハンドリング | -1 |
| 横断 | 変更容易性・保守性 | -1 |
| 横断 | 開発者体験（DX） | -2 |
| 横断 | セキュリティ・配布可能性 | -2 |
| 横断 | プロジェクト全体設計 | -2 |
| | **合計** | **-36** |

---

## 技術評価層 — 運営側コア

### auth-server

- **`Auth.Token.Keys` が全 JWT 署名を 1 つの GenServer で直列化する（継続）** `-1`
  > `signer/0`・`jwks/0`・`signer_for_kid/1` が毎回 `GenServer.call` を通る。鍵は起動後に変わらないので `:persistent_term` に置けば、直列化とメッセージコピーの両方が消える。現規模では顕在化しないが、login / refresh のスループットの上限がここで決まる。
  > 改善方針: `handle_continue(:load_keys)` の最後で `:persistent_term.put/2`、読み取り関数は `:persistent_term.get/1` に置換（新規依存なし）。
  > 対象ファイル: `auth-server/lib/auth/token/keys.ex`（L16-24）

### assets-server

- **BLAKE3 を外部 CLI `b3sum` と一時ファイルで計算する（継続）** `-1`
  > アップロードごとに一時ファイルの書き出しと外部プロセス起動が発生する。改善計画 P3-7 で試行したが「CI が完了しなくなる」ため見送りとなった（`improvement-plan-2026-10-28.md` P3-7）。同じパターンが world の `manifest-seal-hash` CLI にも広がった（world-server の項）。
  > 改善方針: Rust 側に「stdin からハッシュを返す常駐ポート（Port）」を置けば、NIF の脆さを避けつつプロセス起動と一時ファイルを消せる。
  > 対象ファイル: `assets-server/lib/assets/blake3.ex`（L9-32）

- **`reserved_bytes` の期限切れ回収が無く、予約後クラッシュで Quota が恒久的に減る（新規）** `-1`
  > x003f の「予約 → TX 外 put → 確定」はよく設計されているが、予約後にプロセスが落ちると `reserved_bytes` が解放されない。保証表は「予約期限切れジョブ」を非保証として明記しており誠実だが、ユーザーから見ると「何もしていないのに容量が減る」不具合になる。
  > 改善方針: QuotaLedger に予約時刻を持たせ、起動時と定期（`Process.send_after`）に古い予約を回収する。
  > 対象ファイル: `assets-server/lib/assets/blobs.ex`（L104-131, L509-516）, `.workspace/0_doc/architecture/assets-server.md`（L28）

### world-server

- **world に rate limit が一切無く、Room 作成数の上限も無い（継続・範囲拡大）** `-2`
  > `router.ex` のどのパイプラインにも rate limit が無い。認証済みユーザーは `POST /rooms` を無制限に叩けて SQLite が伸び続け、`POST /rooms/:id/tokens` は呼ぶたびに auth への S2S（AuthAccess）を発生させる（world → auth への負荷増幅）。改善計画 P3-7 は CI 問題で見送られたままで、前回 -1 より影響範囲が広いことが分かったため -2 とした。
  > 改善方針: auth の `Auth.RateLimit`（ETS 固定窓）を `shared/` パッケージに切り出し、world の `:user_auth` 後段に適用。Room は owner ごとの上限を DB 制約で持つ。
  > 対象ファイル: `world-server/lib/world_web/router.ex`（L12-27）, `world-server/lib/world/rooms.ex`

- **共有 JWKS キャッシュが取得失敗時にクールダウンせず、GenServer 内で同期取得する（継続）** `-1`
  > `refresh_and_lookup_kid/2` は「成功後のみクールダウン」で、auth が落ちているときは未知 kid のたびに再取得する。`receive_timeout: 2_000, retry: false` で上限は 2 秒に抑えられたが、GenServer 内の同期 HTTP なので待っている全リクエストが直列に並ぶ。auth 障害が world / assets の全リクエスト遅延に増幅される構図は変わっていない。
  > 改善方針: host / client と同型の SWR（`Task` でバックグラウンド取得し、失敗時も短い負のクールダウン）にする。
  > 対象ファイル: `shared/alchemy_auth_client/lib/alchemy_auth_client/jwks.ex`（L143-170, L181-193）

- **ManifestSeal 発行ごとに一時ファイル + 外部 CLI を起動する（新規）** `-1`
  > x0039 の canonical 化は正しい判断だが、ハッシュ計算を `manifest-seal-hash` CLI に委ね、要求のたびに JSON を一時ファイルへ書いて `System.cmd` を呼ぶ。host の seal 更新は低頻度なので現状は問題にならないが、assets の `b3sum` と同じ負債を world にも持ち込んだ。また CLI がコンテナに無いと seal が出ず全 host が配信停止になる（fail-closed として正しいが、`/health` では検出できない）。
  > 改善方針: 上記 Port 常駐化に統合し、`/health` に CLI 可用性を含める。
  > 対象ファイル: `world-server/lib/world/manifest_seal_hash.ex`（L16-47）

---

## 技術評価層 — 共通契約層

### protocol/

- **参加者上限の天井 `USER_ROOT_SLOTS_MAX = 4` が protocol 定数のまま（部分解消）** `-1`
  > x004d で部屋ごとの容量は Manifest の user_root entries から導出されるようになった（`user_root_slots_from_entries`）。しかし entity ID 帯 `3..=6` と天井 4 は protocol 定数で、5 人以上の部屋は protocol 更新と全バイナリ再ビルドを要する。client.md も「5人以上」を非保証にしている。帯の規則（先頭 ID と連続性）だけを protocol に残し、幅は Manifest で配るのが層の原則に合う。
  > 対象ファイル: `protocol/src/lib.rs`（L56-67, L439-470）, `protocol/src/decode_limits.rs`（L58）

---

## 技術評価層 — ホストユーザー端末

### client/host-server

- **`main()` が約 450 行の一枚岩（部分解消）** `-1`
  > x0042 で `put()` は network worker に移り、「Zenoh の一時失敗でプロセス終了」は解消した。しかし `main()`（L735-1187）は起動・購読・seal 初期化・物理ループ・Presence 回収・ログを 1 関数に持ち続けている。物理ループ本体を `fn tick(&mut HostLoop, now)` に切り出せばユニットテスト可能になる。
  > 対象ファイル: `client/host-server/src/main.rs`（L735-1187）

- **`room_token.rs` が 2000 行超で、JWKS・Room Token 検証・資格情報チェーン・seal キャッシュ・credential worker を同居させる（新規）** `-1`
  > ファイル名は「Room Token」だが、moduledoc 自身が x0021 / x0040 / x003d / x0039 の 4 つの責務を列挙している。「保証の単位でファイルを分ける」というプロジェクト規則に反し、JWKS の hard stale を直すつもりで credential worker のバックオフに触れる、といった変更影響の予測が難しくなる。
  > 改善方針: `jwks_cache.rs` / `room_token.rs`（verify のみ）/ `credential_chain.rs` / `credential_worker.rs` / `seal_cache.rs` に分割。
  > 対象ファイル: `client/host-server/src/room_token.rs`（L1-16, 全 2017 行）

- **bind の host 全体バケットを未知鍵の洪水で枯渇させ、正規ユーザーの入室を止められる（新規）** `-1`
  > x0040 の順序設計（Ed25519 の後・JWT の前に host 全体 20 / 毎秒 10 のバケットを消費）は、JWT 検証の CPU を守る点では正しい。ただし Ed25519 鍵は誰でも無料で作れるので、Zenoh に到達できる peer（WAN では guest 証明書保持者）が毎秒 10 件以上のユニーク鍵 bind を流すと、全体バケットが常に空になり正規ユーザーが bind できない。非保証欄の「未 bind steady の Ed25519 洪水」とは別の、入室 DoS の経路である。
  > 改善方針: 全体バケットを「送信元（Zenoh の source id / 証明書）」単位に分けるか、JWT 検証済みの成功 bind で全体バケットを返却する。
  > 対象ファイル: `client/host-server/src/intent_rate.rs`（L7-12, L27-29）

- **配信 tick ごとに `Vec<(u32, Pose)>` を新規確保する（新規）** `-1`
  > 物理ループの net tick で `Vec::with_capacity(...)` を毎回作り、`FrameSample` として worker に渡している。30Hz なので実害は小さいが、実装ルールの「フレーム間でアロケーションを回さない」に反する。worker との受け渡しをダブルバッファ（返却チャネル）にすれば確保は起動時だけで済む。
  > 対象ファイル: `client/host-server/src/main.rs`（L1120-1135）

### client/app

- **register が `"tos_agreed": true` を固定で送る（継続）** `-1`
  > 利用規約への同意をユーザーが操作していないのに、同意したことになる。前回から変化が無い。Home の登録パネルにチェックボックスを置き、未チェックなら送信しない。
  > 対象ファイル: `client/app/src/auth.rs`（L150）

- **`main.rs` が 1899 行に肥大し、購読スレッド・資格情報・Leave 経路が同居する（新規）** `-1`
  > 前回比 +1290 行（diff 集計）。Manifest / Frame / Presence の購読スレッド、Token 更新、Leave、assets 取得、起動がすべて `main.rs` にある。x0038 / x003a のゲートは関数化されているが、`spawn_manifest_subscriber` は 1 関数で 100 行を超える。`bus/manifest.rs` / `bus/frame.rs` / `bus/presence.rs` / `assets_fetch.rs` に分けるべき。
  > 対象ファイル: `client/app/src/main.rs`

### launcher/

- **host の crash 再起動後、client 側の IPC アドレスが古いまま（継続）** `-1`
  > launcher.md が非保証として明記している。client の「Exit Client+Host」が再起動後の host に届かない。launcher が client に新アドレスを渡す経路（ファイル監視や launcher 経由の中継）が要る。
  > 対象ファイル: `launcher/src/main.rs`, `.workspace/0_doc/architecture/launcher.md`（L16）

---

## 技術評価層 — 転送・公開エッジ

### zenoh-router / edge / relay-agent

- **WAN ACL がワイルドカード subscribe を許し、`wire_cap` による部屋隔離が成立しない（新規・重大）** `-3`
  > `config.wan.json5` の guest / host ルールはキー式 `alchemy/dev/room/*/*/manifest` 等を許可する。Zenoh の ACL は「宣言したキー式がルールのキー式に含まれるか」で判定するので、ルールと同一のワイルドカード式で `declare_subscriber` すれば許可される。そのため、正規の guest 証明書を 1 枚持つ peer は、全部屋の manifest / frame / presence / snapshot と liveliness（`alive/{sub}/{jti}`）を購読でき、受信サンプルのキーから全部屋の `wire_cap` を読み取れる。結果として、(1) `private` / `friends` 公開範囲の部屋のシーン・姿勢・参加者 `sub` が WAN の任意の guest に見える（world の visibility ACL を transport 層で迂回）、(2) 誰でも部屋を作れば `alchemy-host` 証明書を得られるので、漏れた `wire_cap` 宛てに他人の部屋の manifest / frame を put できる（アプリ署名で破棄されるが、DoS 負荷になる）。x004e の保証表「`wire_cap` を知らない peer は他 room キー式に到達できない」は前提（`wire_cap` が秘密であること）が崩れている。
  > 改善方針: 短期は「subscribe も put も wildcard を含まない宣言だけを許す」ことは Zenoh ACL では表現できないため、(a) Relay Agent を L7 プロキシにして room 単位の購読を強制する、または (b) payload を Room 鍵（Room Token 応答で配布）で AEAD 暗号化し、キー式隔離に依存しない。いずれにせよ「guest 証明書で `*/*/manifest` を subscribe し、他 room のサンプルが届かない」否定系テストを WAN 構成で CI に入れる。
  > 対象ファイル: `zenoh-router/config.wan.json5`（L37-97）, `protocol/src/lib.rs`（L323-325, L409-415）, `.workspace/roadmap/slices/x004e-relay-agent-mvp.md`（L13-19）, `.workspace/0_doc/architecture/trust-boundary.md`（L38, L48）

- **Relay Agent が平文 HTTP で秘密鍵と Allocation Token をやり取りし、手順書が `0.0.0.0` 待受を指示する（新規）** `-2`
  > relay-agent はサーバー側で client の秘密鍵を生成し、`key_pem` を JSON で返す。既定は loopback だが、WAN の client / host は VPS 上の relay に届く必要があるため、手順書は `RELAY_AGENT_LISTEN=0.0.0.0:7450` を指示している（コード自身が「cleartext HTTP で秘密鍵を返す」と警告を出す）。Cloudflare Tunnel の ingress にも relay は無い。経路上の盗聴者は Allocation Token と mTLS 秘密鍵の両方を得られ、mTLS の意味が失われる。
  > 改善方針: (1) client が鍵を生成して CSR だけを送る（秘密鍵をネットワークに出さない）、(2) relay を Tunnel ingress（`relay.example.tld`）か TLS 終端の背後に置く、の 2 点。
  > 対象ファイル: `relay-agent/src/main.rs`（L95-103, L219-288, L391-401）, `.workspace/0_doc/manual/zenoh-wan-conoha.md`（L64）, `edge/cloudflared/config.yml.example`

- **client 証明書の NotAfter が 1 日で、失効手段が無い（新規）** `-1`
  > `expires_in=300` は「再取得の SLA」に過ぎず、証明書自体は `-days 1` で 24 時間有効。Room Token が失効しても、退室・BAN されても、証明書は 1 日使える。CRL / OCSP も無い。openssl CLI の制約（日単位）が理由として書かれているが、`-not_after` / `x509 -req -days` の代わりに `openssl ca -enddate` を使えば分単位の NotAfter が出せる。
  > 対象ファイル: `relay-agent/src/main.rs`（L22-27, L258-278）, `relay-agent/README.md`

- **prod compose が auth / world / assets を `0.0.0.0` に公開する（新規）** `-1`
  > `ports: - "4002:4002"` 等はホストの全インターフェースに bind し、Linux の Docker は ufw を迂回する。Tunnel ingress の `/internal/*`・`/dev/*` 404 は直アクセスでは効かない（`/internal` は共有トークンで守られるが多重防御が崩れる）。LAN 公開範囲のために LAN からの直アクセスが要るという事情は理解できるが、それなら `BIND_ADDR` を env にして既定を `127.0.0.1` にし、LAN 運用時だけ広げるべき。postgres だけ `127.0.0.1:5433` に絞っているのと一貫していない。
  > 対象ファイル: `world-server/docker-compose.prod.yml`（L18-19）, `auth-server/docker-compose.prod.yml`（L37）, `assets-server/docker-compose.prod.yml`（L34）

---

## 横断評価層

### テスト戦略

- **relay-agent が CI・`make ci`・`cargo deny` のすべての対象外で、テストが 0 件（新規・launcher と同型の再発）** `-2`
  > mTLS 証明書を発行するセキュリティ境界のコンポーネントなのに、`.github/workflows/` のどの workflow も `relay-agent/**` を path に含まず、`.scripts/ci.sh` にも無い。当日 `cargo test` は 0 件、`cargo clippy -D warnings` は通過した。`verify_allocation`（iss / aud / relay_id / role）・HTTP パーサ（Content-Length / 64KiB 上限）は純粋関数に切り出してテストしやすい。前回 launcher で指摘し x0047 で直した「workspace 外クレートが CI から漏れる」とまったく同じ型の再発で、スライス完了条件に「新クレートは CI に入っていること」が無い。
  > 対象ファイル: `relay-agent/src/main.rs`, `.github/workflows/rust-ci.yml`（L4-15）, `.scripts/ci.sh`（L156-176）

- **adversarial E2E が host の生存だけを見ており、client の discard と WAN mTLS / ACL を通していない（部分解消）** `-1`
  > x0046 で偽 Manifest・seal 無し stale-rev・oversize Input / Manifest を流し、host が Frame を出し続けることを確認するようになった（前回 -1 の「正常系 1 本」は解消）。しかし前回 -3 の本体だった「client が偽 Manifest で退室しない」は E2E では未検証で（moduledoc が「client gen/rev gate / GUI discard は unit」と明記）、E2E の zenohd は tcp・ACL オフなので x004e の mTLS / ACL は一度も CI で動いていない。上記の wildcard 問題はこの欠落が原因で見つかっていない。
  > 対象ファイル: `client/e2e-probe/src/main.rs`（L1-12, L434-519）, `docker-compose.e2e.yml`

- **旧 smoke スクリプトが現行ログ文言と乖離して腐っている（新規）** `-1`
  > 例: `smoke-host-token.sh` は `host Room Token acquired and verified` を grep するが、この文言は現行コードのどこにも無い（当日 `client/` 全体で 0 件）。必ず FAIL するスクリプトが README / Makefile から到達可能なまま残っている。前回の「前提が無いと exit 0」とは逆向きの腐敗で、どちらも「CI に入っていない smoke は腐る」ことを示す。
  > 改善方針: CI に入れない smoke は削除するか、`e2e-probe` のサブコマンドに吸収する。
  > 対象ファイル: `.scripts/smoke-host-token.sh`（L15）, `.scripts/smoke-*.sh`

### 可観測性

- **相関 ID / 分散トレースが無く、auth / world にメトリクスが無い（部分解消）** `-2`
  > x0049 で host / client / assets に loopback 専用の Prometheus テキストが入ったのは前進（強み参照）。しかし 1 回の入室（auth login → world token → relay allocate → host bind → client manifest 適用）を跨いで追う手段は依然として無い。Room Token の `jti` という自然な相関キーがあるのに、world の Logger metadata は `[:request_id]` のみで、host のログにも jti は出ない。auth / world は LiveDashboard 用 telemetry のみで外部スクレイプ不可。
  > 対象ファイル: `world-server/config/config.exs`（L42-43）, `auth-server/config/config.exs`（L95-96）, `client/host-server/src/metrics.rs`（L1-5）

### エラーハンドリング

- **relay-agent が lock / パス変換で `expect` / `unwrap` を使う（新規）** `-1`
  > `self.cache.write().expect("jwks lock")`、`key_path.to_str().unwrap()` など。リクエストごとのスレッドなので panic はそのスレッドだけで済むが、`RwLock` が poison されると以後の全 allocate が panic する（host-server は poison を `exit(78)` で明示的に扱うのに、relay は暗黙）。
  > 対象ファイル: `relay-agent/src/main.rs`（L180, L188, L196, L245-276）

### 変更容易性・保守性

- **DemoScene の手動同期が 3〜4 箇所に増えた（継続）** `-1`
  > Elixir `World.DemoScene`（正本）、host の `sim.rs` テスト用フィクスチャ、`shared-sim/src/demo.rs`、さらに x0039 で `World.CanonicalManifest` の正規化既定値（`@default_move_speed 4.0` 等）が「host の既定と一致させる」と書かれた。いずれも「keep in sync manually」とコメントで約束しているだけで、機械的な一致検査が無い。
  > 改善方針: `manifest-seal-hash` CLI に「DemoScene JSON → canonical hash」を出させ、Elixir と Rust の両テストで同じ期待値（共有テストベクタ）を照合する。
  > 対象ファイル: `world-server/lib/world/demo_scene.ex`, `world-server/lib/world/canonical_manifest.ex`（L74-93）, `client/host-server/src/sim.rs`（L530-538）, `client/shared-sim/src/demo.rs`（L3-4）

### 開発者体験（DX）

- **README・client.md の現在地とキー表が古い（継続・部分解消）** `-1`
  > README は「現状（〜x004d）」「WAN は TLS」「次の空き ID = x0053」のままで、x004e（mTLS / relay）・x004f（BlobGrant）・x0052 以降と、roadmap の「次 = x0066」に追従していない。client.md のチャネル表はキーが `alchemy/dev/room/{ROOM_ID}/manifest` のままで、x004e で入った `{wire_cap}` セグメントが無い（zenoh-router.md は正しい）。
  > 対象ファイル: `README.md`（L9-18）, `.workspace/0_doc/architecture/client.md`（L50-56）

- **`make ci` が relay-agent・共有 Elixir パッケージ・fuzz を覆わない（新規）** `-1`
  > `ci.sh all` は rust / launcher / protocol / auth / world / assets / prod-compose で、`relay-agent`、`shared/alchemy_auth_client`、`shared/alchemy_client_ip` は GitHub Actions 側にしか（または全く）無い。「ローカル CI が緑 = PR が緑」という単一入口の約束が崩れている。
  > 対象ファイル: `.scripts/ci.sh`（L156-176）, `Makefile`

### セキュリティ・配布可能性

- **依存監査（cargo deny）が client workspace だけで、launcher と relay-agent が対象外（新規）** `-1`
  > `rust-deny` ジョブは `manifest-path: client/Cargo.toml` のみ。relay-agent は `jsonwebtoken` を含む独自の `Cargo.lock`、launcher も独自 lock を持ち、どちらもアドバイザリ照合されない。配布 ZIP には launcher が入る。
  > 対象ファイル: `.github/workflows/rust-ci.yml`（L46-56）, `relay-agent/Cargo.lock`, `launcher/Cargo.lock`

- **mTLS 秘密鍵ファイルを temp に書き、削除せず、Windows では権限も絞らない（新規）** `-1`
  > `allocate()` は呼ぶたびに `client-{nanos}.key.pem` を `%TEMP%/alchemy-mtls` に書く。`0o600` は `#[cfg(unix)]` のみで、Windows（配布の主対象）では既定 ACL のまま。古い鍵の削除も無いので、Token 更新のたびに鍵が溜まる。
  > 対象ファイル: `client/zenoh-connect/src/allocate.rs`（L67-81, L93-102）

### プロジェクト全体設計

- **保証表が `wire_cap` による部屋隔離を過大に主張している（新規）** `-1`
  > x004e スライス・trust-boundary.md・zenoh-router.md が「部屋隔離 = `wire_cap` キー式」を保証側に置くが、上記のとおり `wire_cap` は guest の wildcard subscribe で漏れる。このプロジェクトの最大の長所は「保証する／しない」表とコードの一致なので、ここで過大主張があると文書全体の信頼度が下がる。実装が直るまでは非保証側へ移し、「推測困難な capability（購読者からは見える）」と正確に書くべき。
  > 対象ファイル: `.workspace/0_doc/architecture/trust-boundary.md`（L38, L48）, `.workspace/0_doc/architecture/zenoh-router.md`（L14）

- **`vision.md` が見出しのみ（継続）** `-1`
  > 3 回連続で `# Vision` の 1 行のまま。overview.md が実質的なビジョンを担っているなら、vision.md を削除して overview へ誘導する方が誠実。
  > 対象ファイル: `.workspace/vision.md`
