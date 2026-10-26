# Alchemy — 総合評価レポート（第1評価者 Claude Opus 5.5 / 2026-10-26）

- **評価日**: 2026-10-26
- **評価対象**: `main` @ `683e30f`（PR #41 マージ直後）
- **種別**: 初回評価（前回の評価なし。`evaluation/` 配下に既存のまとめ・アーカイブなし）
- **独立性**: 第2評価者（`evaluation/gpt/`）の文書は一切参照していない
- **詳細**: [プラス点](opus-specific-strengths-2026-10-26.md) · [マイナス点](opus-specific-weaknesses-2026-10-26.md) · [提案](opus-specific-proposals-2026-10-26.md)

---

## 総点

| | 点 |
|:---|---:|
| プラス合計 | **+96** |
| マイナス合計 | **-72** |
| **正味** | **+24** |

## ひとことで言うと

**部品の質は高いが、部品をつなぐ「信頼の境界」がまだ開いている。**
auth-server は個人プロジェクトとしては群を抜いて堅く（refresh のファミリー回転・多軸レート制限・鍵ローテーション）、保証表とコードの一致度もまれに見る水準にある。一方で、Untrusted host・公網 Zenoh・Tunnel 公開という「外に出す」段階に入った x001a / x001b 以降は、身元（liveliness の偽装・トークンの再利用・無認証の shutdown）と公開設定（dev ビルドの公開）の穴が評価を大きく押し下げた。さらに、Room Token を更新しないため入室から 5 分で操作できなくなる不具合と、JWKS の取得を入力スレッドで同期実行しているため world が落ちると入力が止まる問題は、デモの範囲でも顕在化する。

---

## 観点別小計

| ブロック | 観点 | プラス | マイナス | 小計 |
|:---|:---|---:|---:|---:|
| 運営側コア | auth-server | +17 | -4 | **+13** |
| 運営側コア | assets-server | +7 | -5 | **+2** |
| 運営側コア | world-server | +6 | -6 | **0** |
| 共通契約 | protocol/ | +7 | -5 | **+2** |
| ホスト端末 | client/host-server | +11 | -7 | **+4** |
| ホスト端末 | client/app | +12 | -9 | **+3** |
| ホスト端末 | launcher/ | 0 | 0 | **0**（スタブのみ・評価対象外） |
| 転送・エッジ | zenoh-router / edge | +6 | -6 | **0** |
| Optional | billing / market | — | — | 対象外（ディレクトリなし） |
| 横断 | テスト戦略 | +5 | -6 | **-1** |
| 横断 | 可観測性 | +3 | -2 | **+1** |
| 横断 | エラーハンドリング | +2 | -2 | **0** |
| 横断 | 変更容易性・保守性 | +3 | -3 | **0** |
| 横断 | 開発者体験（DX） | +4 | -2 | **+2** |
| 横断 | セキュリティ・配布可能性 | +3 | -10 | **-7** |
| 横断 | プロジェクト全体設計 | +10 | -5 | **+5** |
| | **合計** | **+96** | **-72** | **+24** |

---

## ローカル CI の実行結果

WSL2（Ubuntu-26.04）と Docker Desktop で `.scripts/ci.sh` をターゲット別に実行した。

| ターゲット | 結果 | 内訳 |
|:---|:---|:---|
| `rust` | ✅ PASS | fmt --check / clippy -D warnings / build / test（client 55 件・host 19 件、すべて成功） |
| `protocol` | ✅ PASS（一部 SKIP） | `cargo test` 7 件成功。protoc が PATH に無いため、スキーマ compile は SKIP |
| `world` | ✅ PASS | format / compile --warnings-as-errors / credo --strict（問題 0）/ `mix test` 6 件成功 |
| `auth` | ✅ PASS | 同上 + ecto。`mix test` 107 件成功 |
| `assets` | ❌ FAIL | `mix test` 7 件中 1 件が失敗（`upload download and dedupe quota`：200 を期待したが 302） |

**assets が失敗した原因**: gitignore 済みの `assets-server/.env`（R2 の実際の設定）を docker-compose が読み込み、`config/runtime.exs` が test 環境でも `ASSETS_CAS_BACKEND=s3` を適用するため、`test.exs` の `:local` が上書きされる。テストは実際の R2 バケットへ PUT したうえで、302 を返した。GitHub Actions には `.env` が無いので通るはずだが、「手元と CI で結果が違う」「テストが本番ストレージへ書き込む」という 2 つの問題がある（マイナス点のテスト戦略 `-3`）。同じ実行で、hackney の advisory（EEF-CVE-2026-47069 / 47075）も表示された。

smoke スクリプト（`smoke-*.sh`）は、auth / world / zenohd を起動した状態での手動実行が前提なので、今回は実行していない。

---

## ブロック別の講評

### 1. 運営側コア（auth / assets / world）

**auth-server（+13）** は本リポジトリで最も完成度が高い。refresh token のファミリー回転と再利用検知、ETS による多軸レート制限、thumbprint の `kid` と検証専用鍵によるローテーション、アカウント列挙への対策、`FOR UPDATE` を使ったトークン消費は、Phoenix / Ash の実務水準に達している。弱点は、refresh の回転だけ行ロックが抜けている点と、Tunnel の背後でクライアント IP が潰れてレート制限が全員共有になる点。

**assets-server（+2）** は CAS + Quota + 302 署名 URL という「保証表どおりの最小構成」をきちんと実装しているが、細部に事故の種が多い。特に、Content-Type が JSON のアップロードが黙って空の Blob になる問題（`Plug.Parsers` が body を先に消費する）と、テストが `.env` で本番 R2 に向く問題は、すぐに直すべき。

**world-server（0）** は「案内所・門番」という位置づけで、Room Token の claim 設計は正しい。ただし `role` を呼び出し側が自己申告できるので、誰でも任意の部屋の host トークンを取れる。JWKS の未知 kid による再取得にもクールダウンが無い。門番としては、テスト 6 件はあまりに薄い。

### 2. 共通契約層（protocol）

チャネルの分離（Manifest / frame / presence / input / liveliness）は、Resonite 系の設計と同じ筋の良さがある（+3）。一方、`PROTOCOL_PIN` がワイヤに載っていないので「世代を固定する」という保証が実質的に働いていない。RenderFrame に回転が無いことも、VR 基盤のワイヤ契約としては大きな欠落。

### 3. ホストユーザー端末

**host-server（+4）**: 60Hz / 30Hz の分離、Leave・liveliness・TTL による三重の解放、alg を固定した RS256 検証は良い設計。最大の欠陥は、JWKS の更新を入力スレッドで同期的に行っていること（`-3`）。world が 60 秒以上落ちると全入力が拒否され、パケットごとにタイムアウトなしの HTTP が走る。`is_finite` 以外の値の大きさを制限していないので、overview の「物理発散爆弾を 100% 遮断」はコード上まだ成立していない。

**client/app（+3）**: system_ui が OpenXR を知らない構造（+3）と、実機コントローラの render model（+3）は高く評価する。一方、**Room Token を更新しないため、入室から 300 秒後に入力がすべて捨てられる**（`-3`）。これはデモの最中にも起きる機能上の不具合。XR は ash による 2,503 行の生 Vulkan で、Desktop の wgpu と描画経路が二重になっている。`unsafe` 81 か所に対して SAFETY コメントは約 10 か所で、プロジェクト自身の実装ルールに反している。

**launcher（対象外）**: `.gitkeep` だけ。提案として記録した。

### 4. 転送・公開エッジ（zenoh-router / edge）

設定ファイルと手順書の質は良い（+6）。しかし、**Tunnel の手順書どおりに起動すると、MIX_ENV=dev のサーバーがインターネットに公開される**（`-4`）。`/dev/mailbox`（Swoosh のメールプレビュー）が認証なしで見えるため、「forgot-password → mailbox で再設定トークンを読む → reset」の 3 リクエストで任意のアカウントを乗っ取れる。auth の堅さを、公開設定が丸ごと無効にしている。WAN の Zenoh も平文 TCP で、TLS も ACL も無い（`-2`）。

---

## 横断評価

- **テスト戦略（-1）**: auth の 107 件と Rust の 81 件は良いが、サービスをまたぐ自動 E2E が無い。smoke は前提のサービスが無いと成功扱いで終了し、判定の多くはログ文字列の grep に頼っている。assets のテストは環境に依存して本番 R2 に接続する。
- **可観測性（+1）**: auth の構造化ログと host の周期カウンタはあるが、メトリクスの出力・トレース・サービス間の相関 ID が無い。
- **エラーハンドリング（0）**: anyhow + context とタグ付きタプルで一貫している一方、world と assets は 500 応答に `inspect(reason)` を載せている。Mutex が poison されても黙って空の状態で続行する。
- **変更容易性（0）**: 保証単位のファイル分割はルールどおりで模範的（+3）。ただし JWKS と検証のコードが world と assets でコピーされている。
- **DX（+2）**: `make ci` を単一入口にしている点は良いが、R2 を設定した開発者の環境では通らない。Rust CI にキャッシュが無い。
- **セキュリティ（-7）**: 最も重い観点。Zenoh のバス上の身元が守られておらず、liveliness の偽装で他人を退室させられる（-3）、盗聴した Room Token で 300 秒間なりすませる（-3）、shutdown が無認証（-2）。overview が掲げる「Untrusted host の監査」より前に、「Untrusted peer に対する host の防御」が成立していない。
- **プロジェクト全体設計（+5）**: 文書の規律（+4）とスライスの運用（+3）は卓越している。ただし、コンテンツの定義（Coin・箱・entity ID の範囲）が host のコードに直書きされ、client にも複製されているのは、層間インターフェース原則の「定義 vs 実行」に明確に違反している（-3）。

---

## 同種 OSS との比較

| 比較軸 | 比較対象 | 所見 |
|:---|:---|:---|
| 認証基盤 | Phoenix の gen.auth / Keycloak | refresh の再利用検知・多軸レート制限は gen.auth より上で、Keycloak の基本機能に並ぶ。Friendship / Group / OIDC は未実装 |
| 空間同期 | Resonite（FrooxEngine）/ VRChat | Manifest と姿勢ストリームの分離は同じ設計。回転・補間・スナップショット・ACL が欠けており、これらは商用製品では初日から必須 |
| Pub/Sub 運用 | Zenoh の公式デプロイ例 | zenohd を設定だけで使い、TLS / ACL / downsampling を使っていない。Zenoh の強みを半分しか引き出していない |
| エンジン構造 | Bevy（ECS + wgpu + bevy_openxr） | 固定長配列の SoA は小規模では妥当。XR が wgpu を経由しない点は、Bevy の構成より保守コストが高い |
| ドキュメント | 一般的な個人 OSS | 「保証する／しない」をコード・architecture・スライスの 3 層で一致させているのは、比較対象を探すのが難しい水準 |

---

## 優先して直すべき 5 点（第1評価者の推奨順）

1. **公開経路を本番ビルドに切り替える**: Tunnel で公開するのは release イメージだけにし、`/dev/*` を閉じ、seed を流さない。修正コストは小さく、効果は最大。
2. **Room Token の自動更新**: TTL の 80% の時点で再取得する。access token も refresh token で更新する。
3. **host の JWKS 更新をバックグラウンド化**: 失敗しても古い鍵を使い続け、未知 kid にはクールダウンを設け、ureq にタイムアウトを付ける。
4. **Zenoh 上の身元保護**: liveliness による解放を検証済みのセッションに紐づける。shutdown をローカル IPC に移す。WAN を TLS 化し、ACL を入れる。
5. **assets のテスト隔離と Parsers の修正**: runtime.exs の CAS 設定を test 環境では無視する。`/blobs` で生の body を受け取れるようにする。

---

## 評価の前提と制約

- 旧 Elixir 権威実行（contents / Formula NIF / Phoenix Channel・独自 UDP）を前提とした観点は使っていない。
- 目標にあるだけでコードが無い項目（verifier・Formula VM・Wake-on-Join・OSC・Dual Editor・LOD・空間オーディオ・P2P・テナント名前空間・billing / market・launcher の本実装）は減点せず、提案または対象外とした。
- 実機の HMD での動作、および smoke の縦通し（実際の auth / world / zenohd を起動した状態）は確認していない。OpenXR 関連の評価はコードの読解に基づく。
- 本レポートの数値は第1評価者の独立した判断である。まとめは親エージェントが別途作成する。
