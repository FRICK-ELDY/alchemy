# Alchemy — 提案（0 点）詳細（第1評価者 Claude Opus 5.5 / 2026-11-12）

> 評価対象コミット: `8833821`。いずれも「現時点では存在しないが、実装すれば価値が上がる」前向きな次の一手であり、加点も減点もしない。
> 目標のみでコードが無い項目（verifier 統計・billing / market・Formula / OSC 等）はここに置き、減点していない。第2評価者（`gpt/`）の文書は参照していない。

## 採点基準

| 点数 | 基準 |
|:---:|:---|
| 0 | 現時点では存在しないが、実装すればプロジェクトの価値を高める提案 |

---

## 運営側コア

### world-server

- **scene を「永続定義」と「セッション overlay」に分ける** `0`
  > weaknesses の -3（takeover host が canonical scene を書ける）の根本策。`rooms.scene`（owner / editor だけが書く・PlaceRead の正本）と `rooms.session_overlay`（active host が書く・dormant で破棄・seal には両方を合成）に分ければ、Resonite の「ワールド保存」と「セッション内の一時オブジェクト」の区別に近づく。overlay の上限（entity 数・blake3 数）を world 側で持てば DoS 天井にもなる。
  > 対象ファイル: `world-server/lib/world/rooms.ex`, `world-server/lib/world/canonical_manifest.ex`

- **Room 鍵の push 型配布（epoch ヒント）** `0`
  > 現状は host・全 guest が 15 秒ごとに `POST /tokens`。world が epoch を進めたら、host が署名付き Presence（または専用 `room_key_hint` チャネル）に `epoch` を載せ、guest はそれを見たときだけ Token を取り直す。ポーリング間隔は 60〜120 秒に伸ばせる。署名は既存の host 鍵で足りる（新規依存なし）。
  > 対象ファイル: `client/host-server/src/presence.rs`, `client/app/src/room.rs`

- **owner の kick / ban API と friendship 変更フック** `0`
  > `rotate_room_key/2` を HTTP に出し、kick 対象の `sub` を Room 単位の deny list に入れて Token 発行時に弾く。auth の friendship 解除を S2S で world に通知してローテする。Discord / VRChat の「インスタンスから追放」に相当する体験が作れる。
  > 対象ファイル: `world-server/lib/world/rooms.ex`, `auth-server/lib/auth/`

- **HTTP PoP の nonce（1 回限り）** `0`
  > `x-alchemy-pop-nonce` を署名対象に加え、`{jti, nonce}` を 60 秒 ETS に入れて再利用を拒否。scene append / seal の skew 内リプレイを閉じる。RFC 9449 の `jti` と同じ考え方。
  > 対象ファイル: `world-server/lib/world/http_pop.ex`, `protocol/src/http_pop.rs`

- **verifier Spot-Check のオーケストレーション（目標）** `0`
  > `World.Verifier.AuditEngine` の注入点は既にある。OSS 側は「どの Frame を、いつ、誰が抜き打ちするか」の呼び出し配線だけを持ち、統計・閾値は外部 Proprietary に注入する（ライセンス境界ルールどおり）。
  > 対象ファイル: `world-server/lib/world/verifier/`

- **S2S 連合・World 一覧の連合交換・`alchemy-node-info.json`（目標）** `0`
  > overview ブロック 2 の目標。Origin Cert v1 が内容束縛になったので、連合先で検証できる素地は整った。
  > 対象ファイル: `world-server/`

### assets-server

- **Spatial VCS（Commit / Branch / Diff / Fork）の本体と Quota プラン連動（目標）** `0`
  > 現状は thin peel。Origin Cert v1 の `tree_blake3` を Merkle root に拡張すれば、部分木の差分署名ができる。
  > 対象ファイル: `assets-server/lib/assets/vcs/`

- **scene append 時の Grant 照会 API** `0`
  > `HEAD /api/v1/blobs/:blake3?for=<sub>`（S2S・内部トークン）で world が「host / owner がその blob の Grant を持つか」を確認できるようにする。weaknesses の「hash = capability」の穴を閉じる。
  > 対象ファイル: `assets-server/lib/assets_web/router.ex`

---

## 共通契約層

### protocol/ ・ client/shared-sim

- **XPBD の body-body 接触を host でも有効化し、確保ゼロのソルバへ** `0`
  > `FrameBridge` 側では既に LBVH contact を通している。host でも bounds を渡し、`EpochSheet::clone_from`・彩色の世代マーカー化・LBVH の refit（再構築ではなく AABB 更新）で毎 tick の確保を消す。Bevy の `bevy_xpbd`（現 avian）や Jolt の broadphase と同じ方向。
  > 対象ファイル: `client/shared-sim/src/xpbd.rs`, `client/shared-sim/src/spatial.rs`

- **Formula VM / Dual Representation（目標）** `0`
  > overview ブロック 3 の目標。コードが無いので減点しない。
  > 対象ファイル: なし（目標）

- **audio / tracking / client_info の proto（目標）** `0`
  > `client_info.proto`（can_be_host / CPU 余裕度 / RTT）は x0080 のスマートホスト移譲を「手動 takeover」から「自動昇格」に進める前提になる。
  > 対象ファイル: `protocol/proto/`

---

## ホストユーザー端末

### client/host-server ・ client/app

- **HostSupervisor → host への access JWT push（IPC）** `0`
  > weaknesses -2（GUI 起動 host が約 15 分で止まる）の本対応。既存 `host_ipc` に `SET_ACCESS_TOKEN` を足し、client の refresh worker が更新のたびに push。refresh token は子に渡さない現方針を守れる。
  > 対象ファイル: `client/app/src/host_supervisor.rs`, `client/host-server/src/host_ipc.rs`

- **Frame 補間・jitter buffer（~60–120fps 予測補間）** `0`
  > overview の「③ クライアント予測補間 Tick」。`FrameBridge` の二面スナップショットを client でも使えば、受信 30Hz から描画 90Hz への補間が素直に書ける。
  > 対象ファイル: `client/app/src/bus/frame.rs`, `client/app/src/epoch_view.rs`

- **Desktop / Dash の「イベント → コマンド」分離** `0`
  > weaknesses -2 の肥大の解消策。UI はコマンド enum を出すだけにし、実行器（world_api / inventory / supervisor 呼び出し）を別モジュール・別スレッドへ。テストはコマンド列の期待値で書ける。
  > 対象ファイル: `client/app/src/surface/desktop.rs`, `client/app/src/system_ui/dash.rs`

- **Wake-on-Join・貸し出し host プール・OSC Device Input Edge・LOD ビルダー・空間オーディオ（目標）** `0`
  > いずれも overview の目標でコード無し。減点しない。
  > 対象ファイル: なし（目標）

- **Windows 配布物のコード署名と自動更新** `0`
  > `windows-release.yml` は pre-release ZIP まで来た。SmartScreen 警告を避けるには Authenticode 署名（Azure Trusted Signing 等）が要る。更新は `alchemy://` ディープリンクと合わせて検討。
  > 対象ファイル: `.github/workflows/windows-release.yml`

---

## 転送・公開エッジ

### relay-server / relay-agent / edge

- **mTLS 証明書を部屋に束縛（SAN に room_id / L7 プロキシ）** `0`
  > zenohd の静的 ACL では「host 証明書なら全部屋の `inventory/spawn` 購読」が残る。AEAD で機密性は守れているが、トラフィック解析・DoS の面では部屋束縛が望ましい。zenohd の `access_control` を動的に生成するか、薄い L7 プロキシを relay-agent 側に置く。
  > 対象ファイル: `relay-server/config.wan.json5`, `world-server/lib/world/mtls_issuer.ex`

- **マルチテナント名前空間・CCU / 帯域 QoS（目標）** `0`
  > overview の `alchemy/tenant/{運営者ID}/room/{room_id}/**`。コード無しのため減点しない。
  > 対象ファイル: なし（目標）

---

## Optional Profile

- **BillingProvider / market-server（目標・BSL-1.1）** `0`
  > ディレクトリ未設置のため評価対象外。新設時はライセンス境界ルールどおり直下に BSL-1.1 `LICENSE` を置く。
  > 対象ファイル: なし（目標）

---

## 横断

### テスト戦略・DX

- **CI 用 Elixir ビルドボリュームを開発コンテナと分離** `0`
  > 当日の assets フレーク（credo コンパイル時の型検査クラッシュ）の再発防止。`ci.sh` の `docker compose run` に CI 専用の `_build` / `deps` ボリュームを渡す。
  > 対象ファイル: `.scripts/ci.sh`

- **物理ホットパスの確保回数テスト（カウンタアロケータ）** `0`
  > `#[global_allocator]` をテストバイナリだけ差し替え、`step_dt` 1 回あたりの確保回数を 0 に固定する回帰テスト。新規依存なしで書ける。
  > 対象ファイル: `client/shared-sim/tests/`（新設）

- **プロパティベーステスト（scene CAS・Room 鍵ローテの並行性）** `0`
  > Elixir 側は StreamData（新規依存・要承認）、Rust 側は既存の fuzz を状態機械テストへ拡張。「append / delete / rotate を任意順に並行実行しても revision と epoch が単調」を性質として書く。
  > 対象ファイル: `world-server/test/`

### プロジェクト全体設計

- **スライス完了条件に「文書 grep 更新」を常設** `0`
  > Rapier → XPBD、launcher 退役のような置換のたびに overview / architecture / README / workflow を grep して更新する手順をスライステンプレートに入れる。
  > 対象ファイル: `.workspace/roadmap/slices/`
