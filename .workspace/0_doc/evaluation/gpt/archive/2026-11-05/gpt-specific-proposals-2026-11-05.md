# Alchemy 第2評価者 — 具体的な提案（2026-11-05）

## 採点基準

| 点数 | 基準 |
|:---:|:---|
| 0 | 現時点では存在しないが、実装すればプロジェクトの価値を高める提案 |

## セキュリティ境界

### Room key / relay

- **Room key epoch と revoke-driven rotation** `0`
  > Room に `key_epoch` を追加し、visibility 縮小・ban・host handoff・明示 revoke で新鍵へ回転する。Room Token、AEAD AAD、Manifest に epoch を入れ、host/client は world が署名した epoch 上昇だけを受理する。過去参加者の wildcard 復号を閉じられる。
  > 対象ファイル: `world-server/lib/world/rooms.ex`, `protocol/src/wire_aead.rs`

- **Relay Agent の CSR-only 化または廃止** `0`
  > world CSR issuer と同じ `csr_pem -> cert_pem` 契約へ統一し、private key を API 応答・一時ファイル・サーバーメモリへ持ち込まない。world 発行が標準運用で十分なら Agent を互換層ごと段階廃止する方が境界は小さくなる。
  > 対象ファイル: `relay-agent/src/lib.rs`

- **Allocation の主体別 throughput policy** `0`
  > in-flight semaphore に加え、検証済み `sub` / relay_id / source IP の短窓・長窓 bucket を入れる。`Retry-After`、deny reason、issue latency histogram を既存 loopback metrics に追加する。
  > 対象ファイル: `relay-agent/src/lib.rs`, `relay-agent/src/metrics.rs`

## 永続状態と資源予算

### world / client

- **Scene revision の楽観 CAS** `0`
  > append/delete body に `expected_revision` を要求し、`UPDATE rooms SET scene=?, revision=? WHERE room_id=? AND revision=?` で一意に進める。409 conflict を host worker が scene 再取得後に有限回再試行する。
  > 対象ファイル: `world-server/lib/world/rooms.ex`, `client/host-server/src/room_token/room_scene.rs`

- **GLB decode budget の共通型** `0`
  > `AssetDecodeBudget { input_bytes, expanded_bytes, vertices, indices, image_pixels, nodes }` を client assets 境界に置き、Desktop mesh と OpenXR render model の両方へ適用する。上限違反は placeholder と低 cardinality counter に落とす。
  > 対象ファイル: `client/app/src/mesh.rs`, `client/app/src/surface/xr/render_model.rs`

- **Room Token 操作別 rate pipeline** `0`
  > host_keys / seal / scene mutation / PlaceRead を同じ一律 bucket にせず、署名コスト・DB 書込・読取で上限を分ける。鍵は `room_id+jti` と ClientIp の二軸にし、Token 失効までの乱用を局所化する。
  > 対象ファイル: `world-server/lib/world_web/router.ex`, `world-server/lib/world_web/plugs/rate_limit.ex`

## テストと運用

### E2E / reliability

- **実 host-server を通す WAN 一往復 E2E** `0`
  > world CSR → host credential worker → zenohd mTLS/ACL → signed+AEAD Frame → headless client trust/apply を Compose で通す。no-cert、guest sim put、旧 epoch、wrong room key を同じ fixture で否定する。
  > 対象ファイル: `docker-compose.e2e.wan.yml`, `.scripts/smoke-compose-wan-e2e.sh`

- **Windows filesystem テストの hermetic 化** `0`
  > XR loader のテストを UUID/tempfile ディレクトリへ移し、VDF parse と `Path::is_dir` を別テストにする。Windows native を対応 OS と掲げる以上、再試行で緑になる flake を残さない。
  > 対象ファイル: `client/app/src/surface/xr/loader.rs`

- **SLO dashboard と alert runbook** `0`
  > 既存 loopback metrics を collector に集約し、JWKS hard-stale、cert issue latency、seal failure、Frame age、tick late、CAS orphan backlog を SLO として定義する。公開 scrape はせず、運用 LAN 内 collector に限定する。
  > 対象ファイル: `relay-agent/src/metrics.rs`, `world-server/lib/world/metrics.ex`, `client/host-server/src/metrics.rs`

## 目標機能

### 評価対象外からの次段

- **Verifier Spot-Check の最小 orchestration** `0`
  > 現在の `AuditEngine` 注入点を維持し、world が署名済み snapshot を抽選取得して engine へ渡す orchestration だけを OSS 側に置く。統計閾値・知覚 hash は proprietary 実装へ残し、ライセンス境界を崩さない。
  > 対象ファイル: `world-server/lib/world/verifier.ex`

- **Frame interpolation / jitter buffer** `0`
  > 署名・generation gate 後の pose に小さな時系列 buffer を置き、render tick で補間する。ネットワーク権威を client へ移さず、late frame と host generation 切替時の flush 規則を先に契約化する。
  > 対象ファイル: `client/app/src/scene.rs`, `client/app/src/bus/frame.rs`

- **launcher deep link と handoff は別スライスで進める** `0`
  > 現行 discovery / crash policy を壊さず、`alchemy://` 解決と host handoff を分離する。deep link は入力検証、handoff は world generation CAS と snapshot transfer が完了条件になる。
  > 対象ファイル: `launcher/src/main.rs`

**提案合計: 0**
