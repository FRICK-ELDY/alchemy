# Alchemy — 総合評価レポート（第1評価者 Claude Opus 5.5 / 2026-10-28）

> 評価対象コミット: `6a0a879`（2026-10-28 09:50 JST、PR #64 マージ直後）。前回 `683e30f`（2026-10-26）から 23 本のスライス（x001e〜x0037）がマージされている。
> 評価方法: コードを直接読み、Docker 上で CI ターゲットを個別に実行した。第2評価者の文書・まとめ文書・改善計画は読んでいない。前回の自分の評価（`archive/2026-10-26/`）は解決確認にだけ使い、「解決」と判定した項目は必ず対象ファイルを読み直した。
> 詳細: [プラス点](./opus-specific-strengths-2026-10-28.md) / [マイナス点](./opus-specific-weaknesses-2026-10-28.md) / [提案](./opus-specific-proposals-2026-10-28.md)

## 採点基準

| 点数 | 基準 |
|:---:|:---|
| +5 | このクラスの個人プロジェクトでは見たことがないレベルの卓越した実装 |
| +4 | プロダクションレベルのゲームエンジン・OSSと比較しても遜色ない実装 |
| +3 | 同規模・同種プロジェクトの平均を明確に上回る実装 |
| +2 | 業界の一般的なベストプラクティスに沿った、良い設計判断 |
| +1 | 正しく実装されている。問題はないが特筆するほどではない |
| 0 | 提案（採点に含めない）／目標のみの未実装（減点しない） |
| -1 | 軽微な問題。改善が望ましいが現状でも機能する |
| -2 | 明確な問題。放置すると将来的に技術的負債になる |
| -3 | 深刻な問題。現時点でバグ・パフォーマンス劣化・保守困難を引き起こしている |
| -4 | 致命的な問題。設計の根本的な見直しが必要 |
| -5 | プロジェクトの存続に関わる問題 |

---

## 総合点

### 点数

| 項目 | 点数 |
|:---|---:|
| プラス合計 | **+176** |
| マイナス合計 | **-36** |
| **正味** | **+140** |

> 前回は +96 / -72 = +24。プラスの増加（+80）の一部は、新機能が増えたことに加え、今回は観点を細かく分けて数えたことによる。比較するときは、前回の弱点がいくつ消えたか（マイナス -72 → -36）を主な指標として見てほしい。

### 観点別小計

| ブロック | 観点 | プラス | マイナス | 小計 | 前回 |
|:---|:---|---:|---:|---:|---:|
| 運営側コア | auth-server | +22 | -2 | **+20** | +13 |
| 運営側コア | assets-server | +11 | -2 | **+9** | +2 |
| 運営側コア | world-server | +16 | -2 | **+14** | 0 |
| 共通契約 | protocol/ | +20 | -1 | **+19** | +2 |
| ホスト端末 | client/host-server | +24 | -4 | **+20** | +4 |
| ホスト端末 | client/app | +19 | -4 | **+15** | +3 |
| ホスト端末 | launcher/ | +2 | -1 | **+1** | 0（スタブ） |
| 転送・エッジ | zenoh-router / edge | +10 | -4 | **+6** | 0 |
| Optional | billing / market | — | — | 対象外（ディレクトリなし） | 対象外 |
| 横断 | テスト戦略 | +11 | -4 | **+7** | -1 |
| 横断 | 可観測性 | +3 | -2 | **+1** | +1 |
| 横断 | エラーハンドリング | +4 | -1 | **+3** | 0 |
| 横断 | 変更容易性・保守性 | +8 | -1 | **+7** | 0 |
| 横断 | 開発者体験（DX） | +6 | -1 | **+5** | +2 |
| 横断 | セキュリティ・配布可能性 | +7 | -5 | **+2** | -7 |
| 横断 | プロジェクト全体設計 | +13 | -2 | **+11** | +5 |
| | **合計** | **+176** | **-36** | **+140** | **+24** |

### ひとこと評価

**前回の致命傷はほぼ塞いだが、偽 Manifest 1 パケットで全員を退室させられる穴と、手順書どおりでは起動しない prod 構成という新しい境界の穴を生んだ。**

前回「部品の質は高いが、部品をつなぐ信頼の境界がまだ開いている」と評した点に対して、この 2 日で 23 本のスライスを積み、Ed25519 のワイヤ署名・world による seal と attestation・role から capability への写像・liveliness の jti 束縛・トークンの自動更新・JWKS の SWR・prod 用 compose・ヘルメティック E2E と、指摘のほぼすべてに正面から答えた。保証表とコードの一致度は今回も非常に高い。一方で、新しく作った「検証」の失敗時の振る舞い（退室）と、新しく追加した必須設定（S2S トークン）が既存の経路（prod compose）に反映されていない、という「継ぎ目」の欠陥が新たに出た。

---

## ブロック別講評

### 運営側コア（auth / assets / world）

> **auth-server（+20）**: 前回の弱点だった refresh の競合と Tunnel 背後のクライアント IP がどちらも直り、ソーシャルグラフは DB の CHECK 制約で不変条件を守るなど、個人プロジェクトとしては群を抜いて堅い。残るのは鍵の GenServer 直列化と、`/internal/*` が ingress で塞がれていないことくらい。
>
> **assets-server（+9）**: 前回最も重かった「開発者の `.env` でテストが本番 R2 に接続する」を、runtime・テスト・ci.sh の三重で塞いだ。Quota の条件付き UPDATE、`Plug.Parsers` の撤去、hackney の除去もきれいに決まっている。R2 へのアップロードを SQLite のトランザクション内で行っている点は、実運用の負荷で効いてくる。
>
> **world-server（+14）**: 「名簿と鍵の発行所」として大きく成長した。永続化でストレージ障害と「存在しない」を区別し、障害で認可を誤らない方針を明文化している点は特に良い。HostAttestation / ManifestSeal は、Untrusted host の配る内容を world が認めた時点に縛る仕組みとして秀逸。ただしテストが開発用 DB を読んで落ちる問題（テスト戦略の項）は、前回 assets で直した不具合と同じ型の再発で、学びが横展開されていない。

### 共通契約（protocol）

> **protocol（+19）**: 最も大きく伸びた層。前回は「PIN がワイヤに載らない」「姿勢が無い」「room_id 検証が 2 つ」と弱点が並んでいたが、PIN の搭載・TRS・共有テストベクタ・`wire_crypto` による署名規約の集約・配送意味論の文書化（peer-delivery.md と `decide_frame_tick`）がすべて入った。Zenoh の best-effort 配送の上に「tick の欠落を検出して snapshot で再同期する」整合性を、文書と関数の両方で固定している点は、同種の OSS でもあまり見ない。参加者上限の定数が protocol にある点だけが層の原則に反する。

### ホスト端末（host-server / app / launcher）

> **host-server（+20）**: 前回の「JWKS を入力スレッドで同期取得」「liveliness の偽装で他人を退室させられる」「定義と実行の逆転」がすべて解消した。Intent セッション表は、署名を JWT より先に検証する・`session_pub` の付け替えを拒否するなど、順序の細部まで考えられている。一方、資格情報の更新を物理ループ内で同期実行し、1 回の失敗でプロセスごと終了する作りは、「fail-closed」の意図は正しいが、auth の一時的な障害で部屋が消える。fail-closed は「失効したら止める」であって「失敗したら止める」ではない。
>
> **client/app（+15）**: seal と attestation で host 鍵を確定し、Frame / Presence を署名で選別する信頼チェーンを client 側で閉じたのは大きな前進。トークンの 80% 自動更新で「5 分で操作不能」も直った。しかし、その検証が失敗したときに `leave_room_local()` を呼ぶため、検証そのものが攻撃面になった（セキュリティの項）。描画経路の二重化は解消の見込みが立たないまま `vk_loop.rs` が 2720 行に増えている。
>
> **launcher（+1）**: スタブから実装に進んだ。ready ファイルで host 自身が bind したアドレスを受け取る方式は TOCTOU が無く、終了コードで再起動の要否を分ける判断も妥当。ただし CI の対象外で、当日 clippy が失敗し、テストも無い。

### 転送・エッジ（zenoh-router / edge）

> **（+6）**: 前回 -4 だった「Tunnel 手順書が開発ビルドを公開する」は、release の prod compose・ingress での `/dev/*` 遮断・seed の prod スキップで直った。WAN の Zenoh も TLS になった。ところが、x0035 で auth と world の prod 起動に `AUTH_INTERNAL_TOKEN` が必須になったのに、prod compose と `.env.prod.example` が更新されておらず、手順書どおりに起動すると両サービスが raise で落ちる（-3）。Zenoh の ACL が無いことは、client の「検証失敗で退室」と組み合わさって実害になっている。

### 横断

> **テスト戦略（+7）**: ヘルメティックな compose E2E が GitHub Actions に入り、Rust のテストは 81 件から 166 件に倍増した。モックに逃げず、テスト内で RSA 鍵を作って本物の JWT で bind を通す作りが良い。弱点は、world のテスト DB 汚染、E2E が正常系 1 本だけで「偽 Manifest」などの否定系が無いこと。否定系の E2E があれば、セキュリティの -3 は CI で見つかっていた。
>
> **可観測性（+1）**: 前回から進展が無い。Room Token の `jti` という自然な相関キーがあるのに使っていない。
>
> **エラーハンドリング（+3）**: ストレージ障害の分離が良い。`detail: inspect(reason)` はまだ残っている。
>
> **保守性（+7）**: JWKS の共通パッケージ化、Scene 分類の単一クレート化、unsafe コメントの lint 強制と、前回の指摘に仕組みで答えている。モジュールの非保証欄に自分の弱点を正直に書く習慣は、このプロジェクトの最大の文化的な長所。
>
> **DX（+5）**: Rust CI のキャッシュ、prod compose。`make ci` が手元で world の汚染により赤になるのは新しい参加者にとって痛い。
>
> **セキュリティ（+2）**: 前回 -7 から大きく改善した。release の seed 除去、host_control の既定 OFF、cargo deny のゲートはいずれも正しい。しかし新規の重大項目が 2 つある。(1) 偽 Manifest / Presence による全員の強制退室（-3）、(2) bind の JWT 盗聴によるセッション乗っ取り（-2）。(2) は `session.rs` が自ら非保証として書いている点を評価するが、`cnf` による束縛で塞げる。
>
> **全体設計（+11）**: trust-boundary.md の脅威表、保証表とコードの一致、「評価 → 改善計画 → スライス → PR」の自己改善サイクルが実際に回っていること。定義と実行の逆転は大部分が解消した。

---

## 前回（2026-10-26）との比較

### 解決した主な弱点

| 前回の弱点 | 前回 | 今回の確認内容 |
|:---|---:|:---|
| Tunnel 手順書が開発ビルドを公開する | -4 | prod compose（release）・ingress で `/dev/*` を 404・seed は prod でスキップ |
| liveliness の Delete で誰でも他人を退室させられる | -3 | Delete のキーの jti が検証済み Intent の jti と一致するときだけ、猶予後に解放 |
| 盗聴した Room Token で 300 秒なりすませる | -3 | 定常 Intent は Ed25519 署名 + 単調 seq のみ（bind 時の盗聴は残る。新規 -2） |
| Room Token を更新せず 5 分で操作不能 | -3 | access / Room Token を TTL の 80% で自動更新、liveliness を付け替え |
| JWKS 取得を入力スレッドで同期実行 | -3 | SWR + バックグラウンド更新 + 未知 kid の 1 秒クールダウン |
| 開発者の `.env` で assets のテストが本番 R2 に接続 | -3 | runtime・テスト・ci.sh の三重で隔離 |
| 定義と実行の逆転（host がシーンを持つ） | -3 | world の Scene から `PhysicsWorld::from_entries` で構築（一部残る。-1） |
| PIN がワイヤに載らない / 姿勢が無い | -2 / -2 | Manifest と Presence に PIN、RenderFrame と SceneEntry に TRS |
| `host` ロールを自己申告できる | -2 | owner のみ、未知 role は 422 |
| refresh 回転の競合 / Tunnel 背後で IP 共有 | -2 / -2 | `FOR UPDATE` / 信頼プロキシからの `CF-Connecting-IP` |
| `Plug.Parsers` で空 Blob が保存される / Quota 競合 | -2 / -1 | Parsers 撤去 / 条件付き UPDATE |
| Zenoh が平文 / host_control が無認証 | -2 / -2 | WAN は TLS / 既定 OFF + launcher はループバック IPC |
| サービスをまたぐ E2E が無い | -2 | compose E2E を CI で実行 |
| JWKS コードのコピー | -2 | `alchemy_auth_client` に共通化 |
| unsafe の SAFETY コメント不足 | -2 | `undocumented_unsafe_blocks = "deny"` |
| release に既定資格情報 / mutex poison の握りつぶし / UI スレッドの同期 HTTP / Rust CI キャッシュ無し / `-o` ファイル | 各 -1 | すべて解消 |

### 継続している弱点

auth の鍵の直列化、assets の `b3sum`、`detail: inspect(reason)`、可観測性、描画経路の二重化、`tos_agreed` の固定送信、host の `main()` の肥大化、host の Room Token 否定系テスト、`vision.md`、Room 作成数の上限、共有 JWKS の失敗時クールダウン。

### 新たに見つかった重大な弱点

| 弱点 | 点数 | 概要 |
|:---|---:|:---|
| 偽 Manifest / Presence で全員退室 | -3 | 検証の失敗で `leave_room_local()`。ACL が無いので誰でも 1 パケットで部屋を空にできる |
| prod compose が起動しない | -3 | `AUTH_INTERNAL_TOKEN` / `AUTH_INTERNAL_URL` が compose と `.env.prod.example` に無い |
| world のテストが開発用 DB を読む | -2 | `WORLD_DATABASE_PATH` が test にも効き、`make ci` が手元で赤 |
| host の資格情報更新が同期・1 回の失敗で終了 | -2 | 最悪約 20 秒停止、auth の一時障害で部屋が消える |
| bind 時の JWT 盗聴で乗っ取り | -2 | `cnf` 無し・後勝ちの再 bind |

> 新規の重大項目は、どれも「新しく作った仕組み」と「既存の経路」の継ぎ目で起きている。個々のスライスは丁寧だが、スライスをまたぐ整合（新しい必須 env を prod compose に足す、新しい検証の失敗時の振る舞いを脅威表で見直す、assets の教訓を world に展開する）を確認する工程が足りない。スライスの完了条件に「prod compose で起動する」「否定系 E2E が通る」を加えると、この型の欠陥はまとめて防げる。

---

## CI 結果（2026-10-28 ローカル実行）

| ターゲット | 結果 | 内容 |
|:---|:---:|:---|
| rust（client workspace） | ✅ PASS | fmt / clippy `-D warnings` / build / test（client 86・host 57・shared-sim 3）。cargo-deny はローカル未導入で SKIP |
| protocol | ✅ PASS | 20 件。protoc の構文検査は PATH に無く SKIP |
| auth-server | ✅ PASS | 124 件。format / compile `--warnings-as-errors` / credo `--strict` / hex.audit |
| assets-server | ✅ PASS | 13 件 |
| alchemy_auth_client | ✅ PASS | 9 件 |
| world-server | ❌ FAIL | 49 件中 1 件失敗（`room_controller_test.exs:16` が 409）。原因は開発用 DB の汚染で、`WORLD_DATABASE_PATH` を一時パスにすると 49 件すべて成功 |
| launcher（CI 対象外） | ❌ FAIL | clippy `-D warnings` で `needless_question_mark` 2 件。テスト 0 件 |
| compose E2E / smoke / 実機 HMD | — | 未実行 |
| GitHub Actions | — | `gh` コマンドが無く未確認 |

> GitHub Actions 上の world CI は、新しいコンテナで DB が空なので通る可能性が高い。失敗はローカル固有だが、「日常スタックを起動したことのある開発者の手元では必ず落ちる」ので DX と信頼性の問題として扱った。

---

## OSS との比較

| 観点 | Alchemy | 比較対象 | 所感 |
|:---|:---|:---|:---|
| 認証基盤 | refresh のファミリー回転・行ロック・kid ローテーション・多軸レート制限 | Keycloak / Ory Hydra | 機能の幅は及ばないが、実装している範囲の正しさは同水準 |
| ワイヤ署名と信頼チェーン | Ed25519 の Intent / Frame / Presence 署名、world の seal と attestation | Resonite / VRChat（非公開）、Matrix の event signing | 「Untrusted host の配信内容を中央が認めた鍵に縛る」設計は Matrix の server signing に近い。個人プロジェクトでここまで閉じている例は稀。ただし検証失敗時の振る舞い（退室）は Matrix なら「破棄」 |
| 配送意味論 | latest デルタ + tick 欠落で snapshot 再同期を文書と関数で固定 | Photon Fusion / Netcode for GameObjects | 商用ネットコードと同じ筋。予測と補間はまだ無い（目標のみ、評価対象外） |
| CAS | BLAKE3 + presigned 302 + 条件付き Quota | IPFS / git LFS | 小さく正しい。ハッシュ計算が外部 CLI なのが惜しい |
| 脅威モデル文書 | キー族ごとの偽 publish 被害・防御・非保証の表 | 大手 OSS でも文書化はまちまち | 同規模の OSS より明確に上。表の一部が新しい攻撃面を見落としている |
| 可観測性 | 構造化ログとカウンタのみ | ほぼすべての商用 OSS が Prometheus / OTLP | 明確に遅れている |
| 描画 | desktop は wgpu、XR は生 Vulkan の 2 本 | Bevy（wgpu + OpenXR 統合）、StereoKit | 二重実装は Bevy の `bevy_openxr` のような統合に比べて保守負担が大きい |

---

## まとめ

正味 +140（前回 +24）。マイナスは -72 から -36 へ半減し、前回 -3 以上だった 7 項目はすべて解消か大幅に改善した。プロジェクトは「部品の質は高いが境界が開いている」段階から「境界の仕組みはそろったが、継ぎ目の確認が足りない」段階に進んだ。次の一手は、(1) 検証失敗を「破棄」にする、(2) prod compose に S2S の設定を通して起動確認を CI に入れる、(3) world のテスト DB を隔離する、の 3 つで、いずれも 1 スライスで終わる規模である。
