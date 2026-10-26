# Alchemy — 提案（0点）詳細（第1評価者 Claude Opus 5.5 / 2026-10-26）

> 現時点ではコードに存在しないが、実装すれば価値が上がる「次の一歩」。加点も減点もしない。
> 目標（overview）にあるだけで未実装の項目は、ここに分類した。第2評価者の文書は参照していない。

## 採点基準

| 点数 | 基準 |
|:---:|:---|
| 0 | 現時点では存在しないが、実装すればプロジェクトの価値を高める提案 |

---

## 技術評価層 — 運営側コア

### auth-server

- **Friendship / Group の SSoT** `0`
  > overview で auth の責務とされているが未実装（auth-server.md でも「将来」と明記）。world の 5 段階公開範囲の判定はこれが前提になる。Ash の resource として `Friendship(requester_sub, addressee_sub, state)` と `Group(owner_sub) + Membership` を追加し、JWT には載せずに world からの S2S API で照会させる（JWT の肥大化と失効の遅れを避ける）。
  > 対象ファイル: `auth-server/lib/auth/accounts/`

- **失効情報を下流サービスへ伝える** `0`
  > world と assets は auth の revocation を確認しない（world-server.md で非保証と明記）。access TTL 900 秒の間はログアウト済みのトークンが有効なままになる。Zenoh の `alchemy/auth/revoked` に jti を流す、または `GET /api/v1/auth/revocations?since=` を world が 30 秒ごとに取得する。
  > 対象ファイル: `auth-server/lib/auth/accounts/token_revocation.ex`

### assets-server

- **Upload の署名 URL 直 PUT（client → R2）と、アップロード時の軽量検疫** `0`
  > 現状は 100MB を Phoenix で全量受け取る。presigned PUT にし、完了の通知で BLAKE3 を照合して BlobRef を確定すれば、サーバーの帯域とメモリが 0 になる。あわせて GLB のヘッダ（magic `glTF`・version・chunk 長）を検査し、overview の「軽量ヘッダ検査」の最初の一歩にする。
  > 対象ファイル: `assets-server/lib/assets/cas/s3.ex`, `assets-server/lib/assets/blobs.ex`

- **Vault / Commit（Spatial VCS）の最小モデル** `0`
  > 最初は `Commit(id, parent_id, author_sub, manifest_blake3)` だけで良い。Manifest 自体を CAS の Blob にすれば、Git の tree と同じ構造になる。Origin Cert はその後に `commit_id` への署名として載せる。
  > 対象ファイル: `assets-server/lib/assets/`

### world-server

- **World / Room の永続化と、所有者・公開範囲** `0`
  > 現状の Room は揮発性の MapSet。SQLite（Ash DataLayer）に `World(owner_sub, visibility)` を置けば、マイナス点で挙げた `role` の自己申告の穴も同時に塞げる。
  > 対象ファイル: `world-server/lib/world/rooms.ex`

- **署名付き SceneManifest（world が定義し、host が実行する）** `0`
  > world が Manifest に署名し（Room Token と同じ JWKS で JWS を作る）、host はそれを検証してから Rapier を構築する。client も同じ署名を検証すれば、host が勝手に配置を変えたことを検出できる。層間原則の「server = 定義、client = 実行」をワイヤで担保する仕組みになる。
  > 対象ファイル: `protocol/proto/scene_manifest.proto`, `world-server/lib/world/`

- **verifier の最小版（Spot-Check）** `0`
  > `apps/verifier` は未実装なので評価対象外。最初の段階としては、frame を subscribe して「user_root の 1 tick あたりの移動量が `move_speed × dt × 許容係数` を超えたら警告する」ルールを 1 つ置くだけで、Untrusted host の監査という価値命題を実証できる。
  > 対象ファイル: （新規）`world-server/lib/world/verifier/`

---

## 技術評価層 — 共通契約層

### protocol/

- **途中参加者向けスナップショットの queryable** `0`
  > host が `alchemy/dev/room/{id}/snapshot` を Zenoh の queryable として宣言し、client は入室時に `get` で全量を取得する。こうすれば frame は純粋なデルタに戻せ（箱を毎 tick upsert する回避策を外せる）、Manifest と Presence を 5 秒ごとに再送する必要もなくなる。
  > 対象ファイル: `protocol/src/lib.rs`, `client/host-server/src/main.rs`

- **`buf lint` / `buf breaking` による互換性ゲート** `0`
  > PROTOCOL_PIN を上げるかどうかを、人の判断から機械的な判定にする。`buf breaking --against .git#branch=main` を protocol-ci に入れる。
  > 対象ファイル: `.github/workflows/protocol-ci.yml`

- **量子化と圧縮（目標の「本格ビット圧縮」）** `0`
  > 未実装なので対象外。回転を追加するときに smallest-three（3 × 10bit + 2bit）と、位置の部屋原点からの相対量子化を同時に設計すると、f64 の 24 byte → 約 10 byte になる。
  > 対象ファイル: `protocol/proto/render_frame.proto`

---

## 技術評価層 — ホストユーザー端末

### client/host-server

- **入力ゲートの property-based テストとファジング** `0`
  > `proptest` で任意の `InputEvent` を生成し、「ゲートを通った入力は物理に適用しても座標が有限のまま」を性質としてテストする。`cargo fuzz` で `InputEvent::decode` → ゲートを回す。Untrusted host・Untrusted client の前提を機械的に担保する手段になる。
  > 対象ファイル: `client/host-server/src/finite.rs`, `client/host-server/src/main.rs`

- **Wake-on-Join と 0 人部屋のスリープ** `0`
  > 目標に挙がっているが未実装。Presence が 0 人になったら物理 tick を止め、liveliness の Put（入室）で再開するだけなら、liveliness の監視がすでにあるので数十行で実装できる。
  > 対象ファイル: `client/host-server/src/main.rs`

- **Formula VM** `0`
  > 未実装なので対象外。導入するなら、Manifest からギミックの定義を受け取り、固定 tick で決定的に実行する Pure Rust のインタプリタとして作る。最初は `rhai` などの既存エンジンで、性能と安全性の境界を探るのが現実的。

### client/app

- **スナップショット補間（目標の 60〜120fps の予測・補間）** `0`
  > 現状は受信した姿勢をそのまま描くので、30Hz で動きがカクつく。`host_time_ms` を使って 2 フレーム分をバッファし、`render_time = now - 100ms` で補間する（Source エンジンや Valve の interp と同じ方式）。自分の user_root だけはクライアント予測と再調整を行う。
  > 対象ファイル: `client/app/src/main.rs`（L215-261）, `client/app/src/scene.rs`

- **カメラ相対の描画で f64 の契約を活かす** `0`
  > ワイヤは f64 なのに、受信時点で `as f32` にしている。描画の直前に「f64 のワールド座標 − f64 のカメラ座標 → f32」に変換すれば、大きなワールドでの jitter を避けられ、binary64 の契約に実質的な意味が生まれる。
  > 対象ファイル: `client/app/src/main.rs`（L237-247）

- **OSC 受信エッジ（Device Input Edge）** `0`
  > 未実装なので対象外。`rosc` で `localhost:9000` を受け、`is_finite` を通してから圧縮した Protobuf に変換する。host を経由させない tracking チャネルとして protocol に `TrackingFrame` を追加する。

---

## 技術評価層 — launcher / 転送・公開エッジ

### launcher/

- **プロセス監督の最小実装** `0`
  > `launcher/` は `.gitkeep` だけなので評価対象外。最初の段階としては、host と client を子プロセスとして起動し、host の死活は Zenoh の liveliness（host 自身のトークン）で監視し、client を閉じても host は残す。shutdown はローカル IPC で送る（`host_control` の無認証問題の解消も兼ねる）。`alchemy://` のディープリンクは OS ごとの登録が必要なので後回しにする。
  > 対象ファイル: （新規）`launcher/`

### zenoh-router / edge

- **本番用 compose プロファイルと Cloudflare Access** `0`
  > `docker-compose.prod.yml`（release イメージ・`MIX_ENV=prod`・seed なし・`TRUSTED_PROXIES` 設定済み）を用意する。運営向けの `/dev/*` は Cloudflare Access の背後に置く。
  > 対象ファイル: `edge/cloudflared/`, `*/docker-compose.yml`

- **テナントの名前空間と QoS** `0`
  > 目標の `alchemy/tenant/{運営者ID}/room/{room_id}/**` は未実装なので対象外。移行するときは、protocol のキー関数に `tenant` 引数を追加するだけで、host と client のコードはほぼ変えずに済む（キー式ヘルパに集約している効果）。zenohd の `downsampling` と `access_control` で、プラン別の上限を表現できる。
  > 対象ファイル: `protocol/src/lib.rs`, `zenoh-router/config.wan.json5`

### Optional Profile

- **BillingProvider / market-server** `0`
  > リポジトリに存在しないので評価対象外。

---

## 横断評価層

### テスト戦略

- **compose を使った縦の E2E を CI に入れる** `0`
  > auth / world / assets / zenohd / host を起動し、ヘッドレスのテスト client（Rust の `#[test]` から Zenoh で input を送り、frame と presence を assert する）で、「login → 入室 → 移動 → Leave で解放 → 不正トークンを拒否」を毎 PR で回す。
  > 対象ファイル: `.github/workflows/`

### 可観測性

- **host のメトリクスを Zenoh の admin キーで公開する** `0`
  > `alchemy/dev/room/{id}/@metrics` を queryable として宣言し、tick の処理時間・入力の拒否数・Presence 数を返す。将来の verifier や運営ダッシュボードがそのまま使える。
  > 対象ファイル: `client/host-server/src/main.rs`

- **物理 tick のベンチマーク** `0`
  > `criterion` で `PhysicsWorld::step` と、デルタ構築の 1 tick あたりの時間を測り、回帰を CI で検出する。ユーザー数・箱の数を増やす前に基準線を作っておく。
  > 対象ファイル: `client/host-server/src/sim.rs`

### 開発者体験（DX）

- **devcontainer と `just` への統一** `0`
  > WSL・Docker・zenohd・b3sum・protoc を含む devcontainer を置き、`make` と多数の `smoke-*.sh` を `just` のレシピに統合して、`just --list` で一覧できるようにする。

### セキュリティ・配布可能性

- **`cargo deny` / `mix_audit` / `sobelow` と SBOM** `0`
  > ライセンスと advisory のゲートを全 workflow に入れ、リリース時に CycloneDX の SBOM を添付する。

- **配布パッケージと署名** `0`
  > launcher の実装とあわせて、Windows のインストーラ（cargo-wix / MSIX）とコード署名を用意する。未実装なので対象外。

### プロジェクト全体設計

- **ADR（Architecture Decision Record）** `0`
  > 「b3sum を CLI で呼ぶ」「XR は ash の生 Vulkan で書く」「Blob は capability として扱う」といった判断を、スライス文書とは別に `0_doc/adr/NNNN-*.md` として残す。次回の評価で、意図した設計なのか負債なのかを判別できるようになる。

- **改善サイクル（improvement-plan）の運用開始** `0`
  > 今回が初回の評価で、improvement-plan はまだ無い。まとめの作成後、マイナス点を Slice（`x001e` 以降）に変換し、次回の評価でその解決状況を検証する流れを定着させる。
