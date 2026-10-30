# Alchemy — 総合評価レポート（第1評価者 Claude Opus 5.5 / 2026-10-30）

> 評価対象コミット: `d2d9f44`（2026-10-30、PR #105 マージ直後）。前回 `6a0a879`（2026-10-28）から 90 コミット。実装は x0038〜x0052、x0053〜x0065 は planned 文書（インベントリラダー等）。
> 評価方法: コードを直接読み、Windows ネイティブ（Git Bash + Docker Desktop）で `./.scripts/ci.sh all` を実行した。relay-agent は別途 `cargo clippy` / `cargo test` を実行した。第2評価者（`gpt/`）の文書は読んでいない。前回の自分の評価と前回まとめ（weaknesses / improvement-plan）は回帰判定の起点にだけ使い、closed と判定した項目はすべて対象ファイルを読み直した。
> 詳細: [プラス点](./opus-specific-strengths-2026-10-30.md) / [マイナス点](./opus-specific-weaknesses-2026-10-30.md) / [提案](./opus-specific-proposals-2026-10-30.md)

## 採点基準

| 点数 | 基準 |
|:---:|:---|
| +5 | このクラスの個人プロジェクトでは見たことがないレベルの卓越した実装 |
| +4 | プロダクションレベルのゲームエンジン・OSSと比較しても遜色ない実装 |
| +3 | 同規模・同種プロジェクトの平均を明確に上回る実装 |
| +2 | 業界の一般的なベストプラクティスに沿った、良い設計判断 |
| +1 | 正しく実装されている。問題はないが特筆するほどではない |
| 0 | 提案（採点に含めない）／目標のみの未実装（減点しない） |
| -1 | 改善余地あり。動作はするが設計・品質上の軽微な問題 |
| -2 | 重要な機能・設計の欠如。放置すると将来の拡張を阻害する |
| -3 | 設計上の明確な欠陥。バグ・クラッシュ・性能劣化を引き起こしうる |
| -4 | プロジェクトの価値命題を損なう重大な欠如。説明責任が果たせない |
| -5 | プロジェクトの根幹を揺るがす致命的な欠陥。存在しないに等しい |

---

## 総合点

### 点数

| 項目 | 点数 |
|:---|---:|
| プラス合計 | **+262** |
| マイナス合計 | **-36** |
| **正味** | **+226** |

> 前回は +176 / -36 = +140。プラスは前回の項目を「継続」として当日再確認したうえで、新規実装分（+86）を積んだ。マイナス合計は前回と同じ -36 だが中身は総入れ替えに近い。前回の -3 以上の 2 項目と、前回まとめの P0 暗号契約 4 項目（-3〜-4）はすべて閉じ、今回の -36 の大半は新しく入った WAN mTLS / relay-agent まわりと、継続の軽微な負債である。

### 観点別小計

| ブロック | 観点 | プラス | マイナス | 小計 | 前回 |
|:---|:---|---:|---:|---:|---:|
| 運営側コア | auth-server | +23 | -1 | **+22** | +20 |
| 運営側コア | assets-server | +20 | -2 | **+18** | +9 |
| 運営側コア | world-server | +23 | -4 | **+19** | +14 |
| 共通契約 | protocol/ | +29 | -1 | **+28** | +19 |
| ホスト端末 | client/host-server | +39 | -4 | **+35** | +20 |
| ホスト端末 | client/app | +34 | -2 | **+32** | +15 |
| ホスト端末 | launcher/ | +4 | -1 | **+3** | +1 |
| 転送・エッジ | zenoh-router / edge / relay-agent | +15 | -7 | **+8** | +6 |
| Optional | billing / market | — | — | 対象外（ディレクトリなし） | 対象外 |
| 横断 | テスト戦略 | +19 | -4 | **+15** | +7 |
| 横断 | 可観測性 | +6 | -2 | **+4** | +1 |
| 横断 | エラーハンドリング | +5 | -1 | **+4** | +3 |
| 横断 | 変更容易性・保守性 | +11 | -1 | **+10** | +7 |
| 横断 | 開発者体験（DX） | +9 | -2 | **+7** | +5 |
| 横断 | セキュリティ・配布可能性 | +11 | -2 | **+9** | +2 |
| 横断 | プロジェクト全体設計 | +14 | -2 | **+12** | +11 |
| | **合計** | **+262** | **-36** | **+226** | **+140** |

### ひとこと評価

**アプリ層の暗号契約は「署名がある」から「署名に意味がある」まで閉じた。残る最大の穴は、新しく入った WAN の transport 層で、`wire_cap` による部屋隔離が wildcard subscribe で成立していないこと。**

前回 P0 の 4 項目（検証失敗での退室・seal の署名 oracle・replay・decode 上限）が、それぞれ専用のスライスと関数とテストを伴って閉じた。host の「失敗で殺す fail-closed」は「失効で配信を止める fail-closed」に設計原則ごと書き換えられ、XR の 2720 行の生 Vulkan 経路は wgpu に統合されて消えた。前回の講評の言葉が、そのまま moduledoc の設計原則に昇格している。一方で、新しく入った relay-agent は、前回 launcher で指摘したのと同じ「workspace 外のクレートが CI から漏れる」型で入り、その WAN 構成は一度も CI で動いていない。結果として、保証表に書かれた「`wire_cap` を知らない peer は他 room に到達できない」が実際には成り立たないことを、誰も検出できなかった。

---

## ブロック別講評

### 運営側コア（auth / assets / world）

> **auth-server（+22）**: 前回以降、lib の変更は ClientIp の共有化と Social の rate limit だけで、堅さはそのまま。残るのは鍵の GenServer 直列化（-1）だけ。Keycloak / Ory Hydra と比べて機能の幅は及ばないが、実装している範囲の正しさは同水準。
>
> **assets-server（+18、前回 +9）**: 今回最も設計が良くなった層。CasObject / BlobRef / BlobGrant の 3 分離、「予約 → TX 外 put → 確定、補償で CAS を消さない」、`access` の fail-closed と 409 `access_conflict`、存在オラクルを「意図」として文書に書く姿勢。git LFS / IPFS の pin と同じ物理と参照の分離を、権利台帳まで含めて小さく正しく作っている。`b3sum` CLI と予約の期限切れ回収の欠如（各 -1）が残る。
>
> **world-server（+19）**: canonical ManifestSeal（+4）が今回の白眉。world が自分の DB の scene と検証済み attestation から署名対象を作るので、seal が「host が言ったもの」ではなく「world が定義したもの」を保証するようになった。弱点は world 全体に rate limit が無いこと（-2）、共有 JWKS の失敗時の直列化（-1）、seal ごとの CLI 起動（-1）。

### 共通契約（protocol）

> **protocol（+28）**: decode 上限（+3）と gen/rev ゲート（+3）を純関数として protocol に置き、host / client が同じ関数を使う。「DoS 境界であって定常配信サイズではない」と運用上の落とし穴まで書く moduledoc の質は高い。参加者の天井 4 が protocol 定数に残る（-1）。

### ホスト端末（host-server / app / launcher）

> **host-server（+35、前回 +20）**: credential worker（+4）・network worker（+3）・`cnf` 照合（+3）・Intent rate の順序設計（+3）と、前回の指摘 4 件を設計の質で上回る形で閉じた。弱点は構造の肥大（`main()` 450 行、`room_token.rs` 2017 行に 5 つの責務）と、bind の host 全体バケットを未知鍵の洪水で枯渇させられる入室 DoS（各 -1）。
>
> **client/app（+32、前回 +15）**: 「検証失敗は discard、退室は明示 Leave のみ」（+4）と XR の wgpu 統合（+4）が大きい。Bevy の `bevy_openxr` と同じ「1 レンダラを複数 surface で使う」形に到達した。`tos_agreed: true` 固定（3 回連続）と `main.rs` 1899 行が残る。
>
> **launcher（+3）**: CI・テスト・再起動ポリシーの純関数化で前回 -1 を解消。IPC stale は非保証として正直に記録されている。

### 転送・エッジ（zenoh-router / edge / relay-agent）

> **（+8）**: mTLS + default deny + role CN ACL（+3）の方向は正しいが、実効性に 3 つの穴がある。(1) guest / host ルールが `alchemy/dev/room/*/*/manifest` のワイルドカード式をそのまま許すので、guest 証明書 1 枚で全部屋のサンプルを購読でき、キーから全部屋の `wire_cap` が読める（-3）。private 部屋のシーンと参加者 `sub` が WAN の任意の guest に見え、world の visibility ACL を transport 層で迂回できる。(2) relay は秘密鍵をサーバー側で生成して平文 HTTP で返し、手順書は `0.0.0.0` 待受を指示する（-2）。(3) 証明書の NotAfter は 1 日で失効手段が無い（-1）。アプリ署名が残るので偽配信は破棄されるが、機密性と DoS 耐性の保証は文書の主張に届いていない。

### 横断

> **テスト戦略（+15）**: cargo-fuzz の CI 化（+3）、adversarial E2E（+2）、「汚染パスを渡しても無視される」ことで隔離を証明するテスト（+2）。Rust のテストは 186 → 251 件。一方で relay-agent は CI・`make ci`・cargo deny のすべてから漏れ、テスト 0 件（-2）。E2E の zenohd は tcp・ACL オフなので、x004e の構成は一度も自動テストされていない。
>
> **可観測性（+4）**: loopback 専用の Prometheus テキストが host / client / assets に入った（+3）。相関 ID とトレースは依然として無い（-2）。
>
> **エラーハンドリング（+4）**: `inspect(reason)` の応答除去で前回 -1 を解消。relay の `expect` が新たな軽微項目。
>
> **保守性（+10）**: ClientIp の共有化で前回 -1 を解消。DemoScene の手動同期が CanonicalManifest の既定値まで広がった（-1）。
>
> **DX（+7）**: `make ci` が Windows ネイティブで全緑（前回は world と launcher が赤）。README / client.md のキー表が古く、`make ci` が relay と共有 Elixir パッケージを覆わない（各 -1）。
>
> **セキュリティ（+9、前回 +2）**: Untrusted host 前提の信頼チェーンが閉じた（+4）。弱点は依存監査が client workspace だけであることと、mTLS 秘密鍵を Windows の temp に権限を絞らず溜めること（各 -1）。transport 層の穴は転送・エッジで計上した。
>
> **全体設計（+12）**: 自己改善サイクルが実際に回っている証拠として、前回の弱点 34 件中 20 件がコード上で closed になった。ただし保証表が `wire_cap` 隔離を過大に主張していること（-1）は、このプロジェクトの最大の長所（保証表とコードの一致）を損なうので重く見るべき。

---

## 前回からの変化（回帰判定 A：コード側）

起点: 自系統 `opus/archive/2026-10-28/opus-specific-weaknesses-2026-10-28.md` と前回まとめ `archive/2026-10-28/specific-weaknesses-2026-10-28.md` / `improvement-plan-2026-10-28.md`。

| # | 前回の弱点（点数） | 判定 | 根拠 / 残件 |
|:---:|:---|:---:|:---|
| 1 | 偽 Manifest / Presence で全員退室（-3） | **closed** | `client/app/src/main.rs` L102-127（ゲート関数は Err を返すだけ）、L336-430（全失敗で `break 'sample` + カウンタ）、`leave_room_local` は L1621 の明示 Leave のみ |
| 2 | ManifestSeal が任意 hash の署名 oracle（まとめ -4） | **closed** | `world-server/lib/world/canonical_manifest.ex` L20-31、`room_controller.ex` L359-370（canonical 計算 → host 値は一致確認のみ） |
| 3 | 署名済み Manifest / Presence の replay（まとめ -4） | **closed** | `protocol/src/generation_gate.rs`、`client/app/src/main.rs` L403-430 |
| 4 | protobuf decode 上限なし（まとめ -3） | **closed** | `protocol/src/decode_limits.rs` L47-63、`client/app/src/main.rs` L349 / L357 |
| 5 | prod compose が S2S env を渡さない（-3） | **closed** | `world-server/docker-compose.prod.yml` L29-32、`auth-server/docker-compose.prod.yml` L48-49、`.scripts/smoke-prod-compose.sh`（当日 PASS） |
| 6 | world テストが開発 DB を読む（-2） | **closed** | `world-server/config/runtime.exs` L58-63、`.scripts/ci.sh` L118-123、`test_env_isolation_test.exs`（当日 65 件 PASS） |
| 7 | 物理ループ内の同期 HTTP・1 回失敗で終了（-2） | **closed** | `client/host-server/src/room_token.rs` L10-13、`main.rs` L983-993 |
| 8 | bind 時 JWT 盗聴で乗っ取り（-2） | **closed** | `client/host-server/src/session.rs` L115-122、`world-server/lib/world/token.ex` L21-29 |
| 9 | CAS put を TX 内で実行 / object と ref の混同（-1 / まとめ -2） | **closed** | `assets-server/lib/assets/blobs.ex` L1-12、L151-221 |
| 10 | 描画経路二重化・vk_loop 2720 行（-2） | **closed** | `vk_loop.rs` 削除（git ls-files に無し）、`surface/xr/mod.rs` L66-69 |
| 11 | asset 失敗を Coin fixture で隠す / HTTP timeout なし（まとめ -2） | **closed** | `client/app/src/main.rs` L95-100、L196-198 |
| 12 | Intent rate limit / JWKS hard stale なし（まとめ -2） | **closed** | `client/host-server/src/intent_rate.rs`、`room_token.rs` L36-41 |
| 13 | Grab 保持中の aim 上限なし（-1） | **closed** | `client/host-server/src/sim.rs` L331、L352 |
| 14 | client world_jwks のクールダウンなし（-1） | **closed** | `client/app/src/world_jwks.rs` L20-25、L218-247 |
| 15 | `/internal/*` ingress 未遮断・Social RL なし（-1） | **closed** | `edge/cloudflared/config.yml.example` L20-23、`auth-server/lib/auth_web/router.ex` L42-44 |
| 16 | launcher が CI 外・clippy 失敗・テスト 0（-1） | **closed** | `.github/workflows/rust-ci.yml` L101-121、`launcher/src/restart.rs`（当日 4 件 PASS） |
| 17 | host Room Token の否定系テストなし（-1） | **closed** | `client/host-server/src/room_token.rs` L2012-（alg / iss / aud / exp / room / role マトリクス） |
| 18 | `detail: inspect(reason)` が応答に残る（-1） | **closed** | world / assets の lib に `detail:` なし（`error_json.ex` の Phoenix 既定を除く） |
| 19 | ClientIp の複製（-1） | **closed** | `shared/alchemy_client_ip/`、両サービスの `client_ip.ex` は委譲のみ |
| 20 | README どおり `make ci` で world が失敗（-1） | **closed** | 当日 `ci.sh all` が ALL PASSED |
| 21 | Zenoh に ACL・クライアント認証なし（-1 / まとめ -2） | **partial** | mTLS + role ACL は入った（`config.wan.json5`）。wildcard subscribe で `wire_cap` が漏れ、部屋隔離は未成立 → 新規 -3 へ |
| 22 | E2E が正常系のみ（-1 / まとめ -2） | **partial** | host 側 adversarial は入った（`e2e-probe` L434-519）。client discard と WAN mTLS / ACL は未通（-1） |
| 23 | メトリクス・トレース・相関 ID なし（-2） | **partial** | host / client / assets の loopback metrics は入った。相関 ID・auth / world metrics は無し（-2 継続） |
| 24 | `main()` 一枚岩・put 失敗で終了（-1） | **partial** | put は network worker へ（x0042）。`main()` は L735-1187 のまま（-1） |
| 25 | USER_ROOT_SLOTS を protocol 固定（-1） | **partial** | 容量は Manifest 由来（x004d）。天井 4 と ID 帯は protocol 定数（-1） |
| 26 | assets GET で他人の Blob を読める（まとめ -3） | **partial** | GET capability を文書化 + `grant_required` を opt-in で実装（x004f）。既定は認証済み capability のまま（案 B 後続・今回は文書と一致するため減点しない） |
| 27 | 定義と実行の逆転の残り（-1） | **partial** | Coin hash は未認証 Home のみに縮小、容量は Manifest へ。DemoScene の手動同期は拡大（-1） |
| 28 | vision.md / README / roadmap の鮮度（-1） | **partial** | roadmap は最新（次 = x0066）。README は x004d 止まり、vision.md は空（各 -1） |
| 29 | `Auth.Token.Keys` の GenServer 直列化（-1） | **open** | `auth-server/lib/auth/token/keys.ex` L16-24（-1） |
| 30 | BLAKE3 を `b3sum` CLI で計算（-1） | **open** | `assets-server/lib/assets/blake3.ex`（-1）。P3-7 は CI 問題で見送り |
| 31 | Room 作成数の上限なし（-1） | **open** | world 全体に rate limit なし（-2 に引き上げ） |
| 32 | 共有 JWKS が失敗時にクールダウンしない（-1） | **open** | `jwks.ex` L143-170。2 秒 timeout で上限は付いた（-1） |
| 33 | register が `tos_agreed: true` 固定（-1） | **open** | `client/app/src/auth.rs` L150（-1） |
| 34 | host restart 後の client IPC stale（まとめ -1） | **open** | launcher.md 非保証（-1） |

**集計: closed 20 / partial 8 / open 6 / superseded 0**（#21 は partial とし、残件を新規の wildcard 弱点として別計上した）。

### 新たに見つかった主な弱点

| 弱点 | 点数 | 概要 |
|:---|---:|:---|
| WAN ACL の wildcard subscribe で `wire_cap` 漏洩 | -3 | guest 証明書 1 枚で全部屋の manifest / frame / presence / liveliness を購読可能。private 部屋の内容と参加者 `sub` が見える |
| relay が秘密鍵を平文 HTTP で返す | -2 | 手順書は `0.0.0.0:7450`。盗聴で Allocation Token と mTLS 鍵の両方が取れる |
| relay-agent が CI・テスト・deny の対象外 | -2 | launcher と同型の再発。テスト 0 件 |
| world に rate limit が一切ない | -2 | 前回 -1（Room 作成上限）を範囲拡大 |

> 前回の講評で「新しく作った仕組みと既存の経路の継ぎ目で欠陥が起きている」と書いた。今回も同じ型が、より大きな新規コンポーネント（relay-agent）で起きた。スライス完了条件に「新クレートは CI・cargo deny に入っている」「新しい保証は否定系 E2E で確認している」を加えることを、改めて強く推奨する。

---

## ローカル CI 結果（2026-10-30）

実行環境: Windows 10.0.26300 / PowerShell から Git Bash で `./.scripts/ci.sh all`、Elixir 系は Docker Desktop 29.8.2。

| ターゲット | 結果 | 内容 |
|:---|:---:|:---|
| rust（client workspace） | ✅ PASS | fmt / clippy `-D warnings` / **cargo deny check（ローカル導入済み・PASS）** / build / test（client 105・host lib 47・host bin 53・shared-sim 3・e2e-probe 0・zenoh-connect 0） |
| launcher | ✅ PASS | fmt / clippy / build / test 4 件 |
| protocol | ✅ PASS | 39 件 + **protoc descriptor コンパイル PASS** |
| auth-server（Docker） | ✅ PASS | 122 件。hex.audit / format / compile `--warnings-as-errors` / credo `--strict` |
| world-server（Docker） | ✅ PASS | 65 件（汚染パス `WORLD_DATABASE_PATH` を渡した状態で） |
| assets-server（Docker） | ✅ PASS | 34 件 |
| prod compose config | ✅ PASS | auth / world の `docker compose config`（`SMOKE_PROD_UP` は未実行） |
| **総合** | ✅ **ALL PASSED** | 前回は world と launcher が FAIL |
| relay-agent（ci.sh 対象外・個別実行） | ⚠️ 0 件 | `cargo clippy -D warnings` PASS、`cargo test` は 0 件 |
| alchemy_auth_client / alchemy_client_ip | — | ci.sh 対象外。GitHub Actions 側のみ（当日未実行） |
| cargo-fuzz | — | Linux / nightly 前提のため Windows では未実行 |
| compose E2E / 実機 HMD | — | 未実行 |

---

## 同種 OSS との比較

| 観点 | Alchemy | 比較対象 | 所感 |
|:---|:---|:---|:---|
| ワイヤ署名と信頼チェーン | canonical seal + attestation + gen/rev 単調 + discard + `cnf` | Matrix（server signing / event auth）、Resonite / VRChat（非公開） | Matrix と同じ「中央が自分のデータから署名対象を作る」「検証失敗は破棄」に到達した。個人プロジェクトでここまで閉じた例は見たことがない |
| transport 隔離 | mTLS + role CN の静的 ACL + キー式 capability | Zenoh 公式の ACL 例、NATS（account / subject 権限）、MQTT broker の topic ACL | NATS の account 隔離や MQTT の per-client topic ACL は「購読側」も絞るのが標準。Alchemy は put 側だけ役割で絞り、購読側の部屋隔離が無い。機密性が要るなら Matrix の Megolm のような payload 暗号化が定石 |
| CAS と権利台帳 | CasObject / BlobRef / BlobGrant + 予約型 Quota | git LFS、IPFS pinning、S3 + DynamoDB 台帳 | 物理と参照の分離・予約と補償の設計は同規模 OSS より明確に上。BLAKE3 計算が外部 CLI なのが惜しい |
| 描画 | wgpu の SceneGpu を desktop と OpenXR で共有 | Bevy（`bevy_openxr`）、StereoKit | 前回の二重実装が解消し、Bevy と同じ構成になった |
| 配送意味論 | latest デルタ + tick 欠落で snapshot 再同期、物理 60Hz / 配信 30Hz、network worker | Photon Fusion、Netcode for GameObjects | 商用ネットコードと同じ筋。予測・補間は目標のみ（対象外） |
| 認証基盤 | refresh ファミリー回転・行ロック・kid ローテーション・多軸 RL | Keycloak、Ory Hydra | 範囲は狭いが正しさは同水準 |
| 可観測性 | loopback Prometheus テキスト（依存なし） | 商用 OSS の大半は Prometheus + OTLP | 前進したが、相関 ID・トレースは依然として遅れている |
| 脅威モデル文書 | キー族 × 被害 × 防御 × 非保証の表、スライス参照付き | 大手 OSS でも文書化はまちまち | 同規模 OSS より明確に上。ただし今回 `wire_cap` 隔離の行が過大主張になっている |

---

## 次にやるべきトップ 5

1. **WAN の部屋隔離を成立させる（-3 の解消）**: 短期は trust-boundary.md / zenoh-router.md / x004e の保証表で `wire_cap` 隔離を非保証側へ移して事実と一致させる。本対応は Relay の L7 化（部屋単位の購読強制）か、payload の Room 鍵 AEAD 暗号化（依存追加は人に回す）。完了条件は「guest 証明書で `alchemy/dev/room/*/*/manifest` を subscribe しても他部屋のサンプルが届かない」否定系テスト。
2. **relay の鍵配布を直す（-2 / -1）**: client 生成鍵の CSR 方式に変え、relay を Tunnel ingress か TLS 終端の背後に置く。証明書の NotAfter を `expires_in` と同じ分単位にする。
3. **relay-agent を CI・`make ci`・cargo deny に入れ、テストを書く（-2 / -1）**: `verify_allocation` と HTTP パーサを純関数化して否定系（iss / aud / relay_id / role / 64KiB）を固定。同時に WAN 構成（mTLS + ACL の zenohd + relay）の compose E2E を追加し、上の 1 の否定系を CI で回す。スライス完了条件テンプレートに「新クレートは CI に入っている」を追加する。
4. **world に rate limit と Room 上限を入れる（-2）**: auth の `Auth.RateLimit` を `shared/` に切り出して world に適用。P3-7 は BLAKE3 と切り離して rate limit 単独のスライスにし、CI 完走の問題を切り分ける。
5. **文書の鮮度と腐った smoke を一掃する（-1 × 4）**: README の現在地（x004e / x004f / x0052 以降・mTLS）、client.md のキー表に `{wire_cap}`、vision.md の記述か削除、`smoke-host-token.sh` のような現行コードと乖離した smoke の削除または `e2e-probe` への吸収。

---

## まとめ

正味 +226（前回 +140）。前回まとめの P0 暗号契約 4 項目と、自系統の -3 以上 2 項目はすべてコード上で閉じ、前回の弱点 34 件中 20 件が closed、8 件が partial になった。アプリ層の信頼チェーンは、Matrix に匹敵する「意味のある署名」まで到達している。マイナス合計が前回と同じ -36 に留まったのは、新たに入った WAN の transport 層（mTLS / ACL / relay-agent）に、部屋隔離が成立しない・鍵が平文で流れる・CI に入っていない、という新しい穴があったためである。次の一手は、この transport 層を「保証表に書いたとおり」にすることで、1〜3 のスライスで終わる規模である。
