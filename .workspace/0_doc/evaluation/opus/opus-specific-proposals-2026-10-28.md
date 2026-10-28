# Alchemy — 提案（第1評価者 Claude Opus 5.5 / 2026-10-28）

> 提案は採点に含めない（すべて `0`）。目標にだけ書かれていて未実装の機能は減点せず、ここで「評価対象外」として扱う。

## 採点基準

| 点数 | 基準 |
|:---:|:---|
| 0 | 提案。採点には含めない |

優先度の目安: **P0** = 次のスライスで着手すべき（マイナス点の重大項目に直結）、**P1** = 近いうちに、**P2** = 規模が大きくなる前に、**対象外** = 目標にのみ記載。

---

## セキュリティ

### P0

- **検証失敗を「退室」ではなく「破棄」にする** `0`
  > Manifest / Presence の pin・room_id・host_pub・seal・attestation のいずれかの検証に失敗したら、そのパケットを捨ててカウンタを増やすだけにする。退室は「検証済みの Manifest で `host_pub` が変わった」場合だけに限る。Presence は pin 比較より先に `host_sig` を検証する。trust-boundary.md の manifest 行の「失敗は leave」も「破棄」に書き換え、e2e-probe に「偽 Manifest を投げても client が部屋に残る」否定系を加える。
  > 対象ファイル: `client/app/src/main.rs`, `.workspace/0_doc/architecture/trust-boundary.md`, `client/e2e-probe/src/main.rs`

- **Room Token に `cnf`（`session_pub` のサムプリント）を結び付ける** `0`
  > client が world に Token を要求するときにセッション公開鍵を送り、world が `cnf.jkt` 相当のクレームに入れて署名する。host は bind 時に `session_pub` と `cnf` の一致を確認する。盗聴した JWT だけでは別の鍵で bind できなくなり、後勝ちの再 bind も同じ鍵に限られる。DPoP（RFC 9449）と同じ考え方。
  > 対象ファイル: `world-server/lib/world/token.ex`, `client/host-server/src/session.rs`

- **prod compose に S2S の設定を通し、起動テストを CI に入れる** `0`
  > 両方の `docker-compose.prod.yml` に `AUTH_INTERNAL_TOKEN` と world の `AUTH_INTERNAL_URL`（コンテナ名で）を `env_file:` か `environment:` で渡し、`.env.prod.example` に追記する。さらに CI で `docker compose -f docker-compose.prod.yml config` と release の起動確認（`/health` が 200）を行えば、同じ型の欠陥を自動で防げる。
  > 対象ファイル: `auth-server/docker-compose.prod.yml`, `world-server/docker-compose.prod.yml`, `.github/workflows/`

### P1

- **Zenoh の `access_control` と mTLS** `0`
  > キー族ごとに publish 権を分け（`…/manifest`・`…/frame`・`…/presence` は host の接続のみ、`…/input` は client のみ）、mTLS で接続元を識別する。署名による破棄と組み合わせれば、偽パケットはルーターの手前で落ちる。
  > 対象ファイル: `zenoh-router/config.wan.json5`

- **Grab 保持中の `aim` を保持距離でクランプする** `0`
  > Grab 開始と同じ 5m の距離を、保持中のオフセットにも上限として適用する。shared-sim の定数をそのまま使える。
  > 対象ファイル: `client/host-server/src/sim.rs`, `client/shared-sim/src/lib.rs`

- **`/internal/*` を ingress で 404 にし、Social API にレート制限を付ける** `0`
  > `/dev/*` と同じ ingress ルールを追加する。Follow / Friendship には `:rate_limited` パイプラインを通す。
  > 対象ファイル: `edge/cloudflared/config.yml.example`, `auth-server/lib/auth_web/router.ex`

- **`mix_audit`（CVE 照合）と `sobelow` を Elixir CI に追加** `0`
  > `hex.audit` は「retired なパッケージ」の検出で、CVE の照合ではない。
  > 対象ファイル: `.github/workflows/*-ci.yml`

### P2

- **デコードと署名検証のファジング（`cargo-fuzz`）** `0`
  > `SceneManifest` / `RenderFrame` / `InputEvent` のデコードから検証までを fuzz の対象にする。
  > 対象ファイル: `protocol/`, `client/host-server/`

- **Room の作成数の上限とレート制限** `0`
  > 対象ファイル: `world-server/lib/world/rooms.ex`

---

## 可用性・性能

### P0

- **host の資格情報更新を専用スレッドに移し、失効前にリトライする** `0`
  > 物理ループは `ArcSwap` などで最新の資格情報を読むだけにする。更新スレッドは TTL の 80% から指数バックオフでリトライし、本当に失効したときだけ Manifest の publish を止める（プロセスは止めない）。
  > 対象ファイル: `client/host-server/src/main.rs`, `client/host-server/src/room_token.rs`

### P1

- **共有 JWKS キャッシュの失敗時クールダウンと非同期取得** `0`
  > 失敗後もクールダウンを効かせ、取得は `Task` に出して GenServer をブロックしない。古い鍵があればそれで検証を続ける（SWR）。client の `world_jwks.rs` にも host と同じクールダウンを入れる。
  > 対象ファイル: `shared/alchemy_auth_client/lib/alchemy_auth_client/jwks.ex`, `client/app/src/world_jwks.rs`

- **assets の CAS put をトランザクションの外に出す** `0`
  > Quota の予約 → トランザクション外で put → メタの確定、の 2 段階にし、put に失敗したら予約を戻す。
  > 対象ファイル: `assets-server/lib/assets/blobs.ex`

- **BLAKE3 のストリーミング計算（Rust NIF / Rustler）** `0`
  > `b3sum` の外部プロセスと一時ファイルを無くす。
  > 対象ファイル: `assets-server/lib/assets/blake3.ex`

- **auth の鍵を `persistent_term` に置く** `0`
  > 対象ファイル: `auth-server/lib/auth/token/keys.ex`

---

## テスト・DX

### P0

- **world のテスト DB を必ず隔離する** `0`
  > `runtime.exs` で `config_env() == :test` のときは `WORLD_DATABASE_PATH` を無視し、assets と同じ隔離テストを書く。`ci.sh` でも明示的に上書きする。Repo を再起動するテストは後始末で Sandbox を `:manual` に戻す。
  > 対象ファイル: `world-server/config/runtime.exs`, `world-server/test/**`, `.scripts/ci.sh`

### P1

- **launcher を CI に入れ、再起動ポリシーを純粋関数にしてテストする** `0`
  > 対象ファイル: `launcher/src/main.rs`, `.github/workflows/`

- **host Room Token 検証の否定系テスト** `0`
  > alg=none / HS256・iss / aud 不一致・期限切れ・room_id 不一致・未知 kid をそれぞれ拒否することをテストする。
  > 対象ファイル: `client/host-server/src/room_token.rs`

- **古い smoke を「前提が無ければ失敗」に統一する** `0`
  > 対象ファイル: `.scripts/smoke-*.sh`

- **`buf lint` / `buf breaking` で proto の互換性を検査** `0`
  > PIN の手動更新に加えて、後方互換を壊す変更を CI で検出する。
  > 対象ファイル: `protocol/proto/`

---

## 可観測性

### P1

- **PromEx / OpenTelemetry（OTLP）の導入と `jti` による相関** `0`
  > auth・world・assets に PromEx を入れ、host のカウンタを Prometheus 形式で公開する。Room Token の `jti` をログのメタデータにすれば、1 回の入室を 4 プロセスにまたがって追える。
  > 対象ファイル: `*/lib/**/application.ex`, `client/host-server/src/main.rs`

---

## 設計・保守性

### P1

- **参加者上限を world の Room 定義から配る** `0`
  > `USER_ROOT_SLOTS` を protocol の定数から外し、Manifest に載せる。protocol は ID 帯の規則だけを持つ。
  > 対象ファイル: `protocol/src/lib.rs`, `world-server/lib/world/demo_scene.ex`

- **`ClientIp` を共有パッケージにする** `0`
  > 対象ファイル: `auth-server/lib/auth_web/client_ip.ex`, `world-server/lib/world_web/client_ip.ex`

- **host の `main()` を「起動」「受信」「物理ループ」「配信」に分割する** `0`
  > 対象ファイル: `client/host-server/src/main.rs`

### P2

- **wgpu-hal による XR 連携で描画経路を 1 本にする** `0`
  > wgpu の Vulkan バックエンドに OpenXR のスワップチェーンを渡し、desktop と XR で同じレンダラを使う。`vk_loop.rs` の 2720 行の大部分を無くせる。
  > 対象ファイル: `client/app/src/surface/xr/vk_loop.rs`

- **`vision.md` を書き、README とロードマップの「現在地」を自動で揃える** `0`
  > 対象ファイル: `.workspace/0_doc/vision.md`, `README.md`

---

## 評価対象外（目標にのみ記載・未実装）

以下はロードマップや overview に目標として書かれているが未実装のもの。減点しない。

- **外部 verifier（検証ノード）** `0`
- **Formula VM（ユーザー定義ロジックの実行環境）** `0`
- **Wake-on-Join（入室時の host 自動起動）** `0`
- **launcher のディープリンク・トレイ常駐・インストーラ** `0`
- **client 側の予測と補間** `0`
- **Group / Block（ソーシャル拡張）** `0`
- **LOD・大規模シーン向け最適化** `0`
- **マルチテナントのキー名前空間** `0`
