# Alchemy プロジェクト総合評価 — 第2評価者 GPT-5.6 Sol

| 評価観点 | プラス | マイナス | 正味 |
|:---|---:|---:|---:|
| auth-server | +10 | 0 | +10 |
| assets-server | +4 | -6 | -2 |
| world-server | +7 | -4 | +3 |
| protocol | +7 | -4 | +3 |
| client/host-server | +9 | -6 | +3 |
| client/app | +9 | -8 | +1 |
| launcher | +2 | -1 | +1 |
| zenoh-router / edge | +3 | -3 | 0 |
| テスト戦略 | +5 | -5 | 0 |
| 可観測性 | +1 | -2 | -1 |
| エラーハンドリング | +2 | 0 | +2 |
| 変更容易性・保守性 | +3 | 0 | +3 |
| DX | +2 | -1 | +1 |
| セキュリティ・配布 | +2 | 0 | +2 |
| プロジェクト全体設計 | +3 | 0 | +3 |
| **合計** | **+69** | **-40** | **+29** |

**評価日:** 2026-10-28  
**評価方式:** architecture の保証／非保証、2026-10-26 の自評価弱点、現行コード・test・CIを直接再検証した。目標のみの verifier / Formula / OSC / billing / market 等は減点していない。  
**総点:** **+69 / -40 = +29点**

## 採点基準

| 点数 | 基準 |
|:---:|:---|
| +1〜+5 | 正しい実装から、個人プロジェクトとして卓越した実装まで |
| -1〜-5 | 軽微な改善余地から、根幹を揺るがす致命的欠陥まで |
| 0 | 未実装・評価対象外、または将来価値を高める提案 |

## エグゼクティブサマリー

> 2日間で前回の重大な trust-boundary 欠陥の大半を実コードとテストで閉じた。任意 host token、無認証 host shutdown、平文 WAN、偽 Frame の直接適用、delta 非収束、巨大 move、seed credential fail-open、quota race、CI advisory 放置は解消され、world 永続/visibility、session signature、attestation/seal、snapshot、launcher、shared-sim、Hermetic E2E まで追加された。個人プロジェクトの改善速度と境界設計は明確に平均以上である。

> ただし「暗号署名がある＝world が内容を承認した」にはまだなっていない。ManifestSeal は untrusted owner host が指定した任意 hash への署名 oracle であり、client は署名済み Manifest/Presence の revision 後退を拒否しない。さらに Zenoh payload は decode size 上限がなく、WAN は server-auth TLS のみで peer ACL がない。CAS GET の owner/grant 認可欠落も残る。現状は堅牢な技術デモへ進んだが、公開 WAN で有料/private UGC を扱う配布版には未到達である。

## 技術評価層

### auth-server — アイデンティティ SSoT

### ✅ プラス点

- **アカウント・鍵・失効 lifecycle** `+4`
  > Argon2、RS256/kid/JWKS、refresh family、password reset/change、deactivate を一つの権威に閉じている。
  > 対象ファイル: `auth-server/lib/auth/accounts.ex`

- **Refresh concurrency の実装修正** `+3`
  > `FOR UPDATE` と条件付き revoke を確認した。前回の二重 rotation は解決済み。
  > 対象ファイル: `auth-server/lib/auth/accounts.ex`

- **Follow/Friendship と S2S access predicate** `+3`
  > 7段階 visibility の関係正本を auth に置き、world は共有 secret 付き判定だけを呼ぶ。
  > 対象ファイル: `auth-server/lib/auth/social.ex`

**小計: +10 / 0 = +10点**

### assets-server — CAS・Quota・門番

### ✅ プラス点

- **local/S3 CAS と presigned GET** `+2`
  > DB に本体を置かず、backend switch と R2直結を実現している。
  > 対象ファイル: `assets-server/lib/assets/cas.ex`

- **原子的 quota 条件更新と test env 隔離** `+2`
  > 前回の quota race とローカルCI環境汚染は解決済み。
  > 対象ファイル: `assets-server/lib/assets/blobs.ex`

### ❌ マイナス点

- **認証済み全員が hash だけで他人の Blob を取得可能** `-4`
  > owner_sub は GET authorization に使われない。private/paid asset を安全に扱えない。
  > 対象ファイル: `assets-server/lib/assets_web/controllers/blob_controller.ex`

- **CAS object と owner reference の混同** `-2`
  > cross-owner dedupe が reference/quota を作らず、CAS side effect の補償もない。
  > 対象ファイル: `assets-server/lib/assets/blobs.ex`

### 💡 提案

- **CasObject / BlobRef / BlobGrant の三層化** `0`

**小計: +4 / -6 = -2点**

### world-server — 案内所・門番

### ✅ プラス点

- **永続 room owner と host role enforcement** `+3`
  > token endpoint は暗黙 create せず、owner だけが host token を取得できる。前回の任意 host role は解決済み。
  > 対象ファイル: `world-server/lib/world_web/controllers/room_controller.ex`

- **7段階 visibility の fail-closed ACL** `+2`
  > private/LAN/social/public と S2S 障害を明確に分ける。
  > 対象ファイル: `world-server/lib/world/room_access.ex`

- **用途別の token/attestation/seal** `+2`
  > audience/typ/room/role を分離し、guest の HTTP capability を拒否する。
  > 対象ファイル: `world-server/lib/world/host_attestation.ex`

### ❌ マイナス点

- **ManifestSeal が任意 body hash の署名 oracle** `-4`
  > DB scene と照合せず host 指定 hash を署名するため、world-authored definition を保証しない。
  > 対象ファイル: `world-server/lib/world_web/controllers/room_controller.ex`

### 💡 提案

- **world が canonical Manifest を構築・署名** `0`

**小計: +7 / -4 = +3点**

### protocol — ワイヤ契約 SSoT

### ✅ プラス点

- **独立 schema・PIN・全 schema CI** `+2`
- **Intent/Frame/Presence の Ed25519 契約** `+3`
- **room validator・delivery・capability の共有** `+2`
  > 対象ファイル: `protocol/src/lib.rs`

### ❌ マイナス点

- **untrusted protobuf の byte/count 上限なし** `-4`
  > decode と repeated allocation が署名検証より先に走り、接続 peer から memory/CPU DoS が可能。
  > 対象ファイル: `protocol/src/lib.rs`

### 💡 提案

- **message class ごとの pre-decode byte limit と post-decode count limit** `0`

**小計: +7 / -4 = +3点**

### client/host-server — Untrusted 計算ホスト

### ✅ プラス点

- **Token→session signature→seq→capability→sanitization** `+3`
  > 巨大 move と遠距離 Grab も test で拒否する。
  > 対象ファイル: `client/host-server/src/session.rs`

- **Rapier 60Hz / delta 30Hz / snapshot** `+3`
  > fixed physics、静止物省略、last-published tick を分離する。
  > 対象ファイル: `client/host-server/src/main.rs`

- **Presence reclaim と credential refresh** `+3`
  > Leave/liveliness/TTL、auth/room/attest/seal refresh、fatal poison policy を実装した。
  > 対象ファイル: `client/host-server/src/main.rs`

### ❌ マイナス点

- **physics loop 内の同期 HTTP/Zenoh wait** `-2`
- **per-session Intent rate limit 不在** `-2`
- **JWKS cache の hard stale expiry 不在** `-2`
  > 対象ファイル: `client/host-server/src/main.rs`

### 💡 提案

- **bounded network worker・token bucket・hard JWKS TTL** `0`

**小計: +9 / -6 = +3点**

### client/app — 体感・描画・UI endpoint

### ✅ プラス点

- **Frame signature/finite/tick/snapshot convergence** `+3`
  > 前回の偽 Frame 適用と delta 非収束は解決済み。
  > 対象ファイル: `client/app/src/main.rs`

- **Manifest seal と HostAttestation 検証** `+2`
- **Desktop/OpenXR と background UI job** `+2`
- **Manifest/shared-sim 駆動 TRS と UBO slot 分離** `+2`
  > 対象ファイル: `client/app/src/scene.rs`

### ❌ マイナス点

- **Manifest/Presence replay protection 不在** `-4`
  > 有効期限内の古い署名済み state を反復適用でき、旧 host key への巻き戻しDoSが可能。
  > 対象ファイル: `client/app/src/main.rs`

- **hash/download failure を fixture で隠す** `-2`
- **主要 HTTP 経路の明示 timeout/cancel policy 不在** `-2`
  > 対象ファイル: `client/app/src/world_api.rs`

### 💡 提案

- **generation/revision gate、error placeholder、jitter buffer** `0`

**小計: +9 / -8 = +1点**

### launcher — プロセスマネージャ

### ✅ プラス点

- **spawn/ready/IPC/monitor/backoff の最小実装** `+2`
  > loopback port 0 + token で process-control plane を Zenoh から分離した。
  > 対象ファイル: `launcher/src/main.rs`

### ❌ マイナス点

- **host restart 後の client IPC address が stale** `-1`
  > 対象ファイル: `launcher/src/main.rs`

### 💡 提案

- **安定 discovery endpoint・deep link・tray・handoff** `0`

**小計: +2 / -1 = +1点**

### zenoh-router / edge — 転送・公開エッジ

### ✅ プラス点

- **WAN TLS 化** `+1`
- **Tunnel の origin秘匿とdeny route** `+2`
  > 対象ファイル: `edge/cloudflared/config.yml.example`

### ❌ マイナス点

- **mTLS・publisher identity・namespace/QoS ACL 不在** `-3`
  > TLS は盗聴を防ぐが、接続 peer 間の key isolation はない。
  > 対象ファイル: `zenoh-router/config.wan.json5`

### 💡 提案

- **Relay Agent と allocation capability** `0`

**小計: +3 / -3 = 0点**

### Optional Profile

> verifier、billing、market-server はコード未設置の目標なので評価対象外。減点しない。

## 横断評価層

### テスト戦略

### ✅ プラス点

- **Hermetic Compose E2E** `+3`
- **境界回帰 test の増加** `+2`
  > protocol 20、Rust workspace 全 test がローカルで成功した。

### ❌ マイナス点

- **world local CI が stale test DB で再実行失敗** `-2`
  > Compose mount 下の `priv/data/world_test.db` を初期化せず、今回49 tests中1 failure（既存`local` roomで201期待が409）となった。
  > 対象ファイル: `world-server/config/test.exs`

- **実 client trust/asset/GPU を通るE2E不在** `-2`
- **fuzz/property corpus 不在** `-1`

### 💡 提案

- **replay/oversize/backpressure/R2 failure の adversarial CI** `0`

**小計: +5 / -5 = 0点**

### 可観測性・デバッグ容易性

### ✅ プラス点

- **理由付き集約ログ** `+1`

### ❌ マイナス点

- **metrics/export/SLO 不在** `-2`

### 💡 提案

- **OpenTelemetry/Prometheus dashboard** `0`

**小計: +1 / -2 = -1点**

### エラーハンドリング戦略

### ✅ プラス点

- **storage/auth/credential failure の fail-closed 分類** `+2`
  > internal error を認可成功や404に潰さず、host invariant poison も停止させる。

**小計: +2 / 0 = +2点**

### 変更容易性・保守性

### ✅ プラス点

- **shared-sim/auth-client と workspace lint** `+3`
  > scene分類、interaction定数、JWKS、unsafe根拠を共有境界へ寄せた。

**小計: +3 / 0 = +3点**

### 開発者体験（DX）

### ✅ プラス点

- **Docker/WSL/個別CI/smoke導線** `+2`

### ❌ マイナス点

- **README の現状・slice表記が stale** `-1`
  > 対象ファイル: `README.md`

**小計: +2 / -1 = +1点**

### セキュリティ・配布可能性

### ✅ プラス点

- **hex.audit / cargo-deny gate** `+2`
  > 前回の dependency advisory 非gateは解決済み。installer/SBOM/signing は未実装目標として提案0点。

**小計: +2 / 0 = +2点**

### プロジェクト全体設計

### ✅ プラス点

- **定義・門番・実行・endpoint・process ownership の分離** `+3`
  > architecture/trust-boundary と実装の一致度は前回より大幅に高い。

### 💡 提案

- **Manifest の「world定義」と「host実行」を暗号契約でも一致させる** `0`

**小計: +3 / 0 = +3点**

## 前回評価（2026-10-26）との比較

### 解決済み

> 対象コードを再読して解決を確認した。

- Refresh Token 二重 rotation: `FOR UPDATE` + conditional revoke
- 任意 user の host role: 永続 owner のみ許可
- room_id validator 不一致: Rust/Elixir shared vector と同じ allowlist
- protoc 対象不足: 4 schema 全て
- 無認証 host_control: 既定OFF + launcher IPC
- 巨大 move/遠距離 Grab: clamp + distance gate
- release の既定 seed credential: release fail-closed
- client の偽 Frame/Presence 適用: host signature/attestation/finite
- delta 順序・再同期: tick gate + snapshot
- box/remote UBO alias:予約 slot 分離
- UI event thread の同期 login/join: background job
- WAN 平文 TCP: TLS
- assets test env 汚染: test runtime isolation
- service 横断 E2E 不在: Compose E2E
- mutex poison の空 Presence 化: fatal stop
- entity分類/ID重複: protocol/shared-sim へ集約
- dependency advisory 非gate: hex.audit/cargo-deny CI

### 未解決・部分解決

- assets owner/grant 認可: 未解決
- quota/CAS 整合性: quota race は解決、object/reference/補償は未解決
- physics loop の同期 publish: 未解決
- fixture fallback: 未解決
- fuzz/property: 未解決
- production metrics: 未解決

### 新たに確認した重大点

1. ManifestSeal が world scene と無関係な任意 hash を署名する。
2. client が署名済み Manifest/Presence の revision replay を拒否しない。
3. Zenoh protobuf payload に decode byte/count limit がない。

## CI・ローカル検証

> Windows上で `bash .scripts/ci.sh` は利用中WSLに `/bin/bash` がなく起動不能だったため、Rust/protoc は native、Elixir は同じ Docker image/command を直接実行した。最初の Docker direct invocation は PowerShell の空 `--entrypoint` 引数解釈で service 名を誤認したため、`--entrypoint sh ... -c` に修正して再実行した。

| 対象 | 結果 |
|:---|:---|
| `protocol cargo test` | **成功** — 20 passed |
| protoc 4 schema compile | **成功** — libprotoc 35.1 |
| `client cargo test --workspace` | **成功** — 全 crate 成功、host-server 57 passed |
| Rust fmt / clippy `-D warnings` / build | **成功** |
| auth Docker mix gate | **成功** — hex.audit/format/compile/credo/migrate、124 tests |
| world Docker mix gate | **失敗** — 49 tests中1 failure。stale `world_test.db` の既存 room で 201期待が409 |
| assets Docker mix gate | **成功** — hex.audit/format/compile/credo、13 tests |
| Compose E2E | repository CI定義とコードを確認。今回のローカル実行は component gate を優先 |

## OSS比較

> Bevy/Godot と比べると renderer/editor/asset pipeline の機能量ではまだ試作だが、auth/world/host/client の trust boundary を明文化し、Room Token→session signature→attested host Frame まで一人で統合した点は同規模の個人プロジェクトを上回る。一方、成熟した game networking/zero-trust relay は payload budget、peer identity、anti-replay、bounded queues、metrics を基本要件にする。Alchemy は暗号 primitive の導入を終えた段階で、次は「誰が何を署名したか」と「資源を何バイト使わせるか」を enforcement する段階である。

## 優先順位

1. ManifestSeal を world DB scene 由来の canonical Manifest に限定する。
2. Manifest/Presence に host generation + monotonic revision gate を入れる。
3. Zenoh 全 message class に byte/count/rate limit を入れる。
4. assets を CasObject/BlobRef/BlobGrant に分け、GET認可を強制する。
5. WAN relay に mTLS identity・room/tenant ACL・QoSを入れる。
6. physics/network worker分離と metrics を導入する。

## 一言評価

> **境界を急速に実装へ落とした強い技術デモだが、署名 oracle・replay・無制限 decode を閉じるまで公開 WAN 基盤とは呼べない。**
