# Alchemy 第2評価者 — 提案（2026-11-18）

## 採点基準

| 点数 | 基準 |
|:---:|:---|
| 0 | 現時点では存在しないが、実装すればプロジェクトの価値を高める提案 |

## 運営側コア

### auth-server / world-server

- **関係 revision + durable revoke outbox** `0`
  > auth の social mutation と同一 DB 境界に outbox を置き、world が冪等 event_id を consume する。`followers_plus` / `friends_plus` は二者通知ではなく影響 owner を再評価し、失敗を retry/メトリクス化する。
  > 対象ファイル: `auth-server/lib/auth/social.ex`, `auth-server/lib/auth/world_notify.ex`, `world-server/lib/world/rooms/room_key.ex`

- **verifier の OSS orchestration** `0`
  > `AuditEngine` Behaviour と Noop 実装、失敗時の host revoke 呼出境界だけを OSS に置く。pgvector・知覚 hash 閾値・Spot-Check 統計は proprietary 注入のまま維持する。
  > 対象ファイル: `world-server/`

### assets-server

- **Origin Cert 鍵履歴と失効 envelope** `0`
  > v2 の `kid` を historical public-key set、revocation、export API と結び、連合ノードが current key 注入なしで offline verify できるようにする。
  > 対象ファイル: `assets-server/lib/assets/vcs/cert.ex`

- **Spatial VCS clone/fetch と PR を現行 accept path上へ追加** `0`
  > push の決定的 object/Grant/ref CAS を再利用し、公開/private RepoRead と shallow fetch を設計する。Save UI は client `vcs-store` を薄い adapter に留める。
  > 対象ファイル: `assets-server/lib/assets/vcs/push.ex`, `client/vcs-store/`

## シミュレーション・配送

### shared-sim / host-server

- **実 host zero-allocation benchmark** `0`
  > Epoch/MVCC の両 backend で 1k entity、500 tick、Grab/接触/dirty publish を含む allocation count・p50/p95/p99 を計測し、MVCC default 切替条件を CI に固定する。
  > 対象ファイル: `client/shared-sim/benches/`, `client/host-server/src/sim.rs`

- **複数 HotPhysicsPage への拡張** `0`
  > 現行単一1024 entity pageを directory + cell page 群へ拡張し、SceneBudget 4096 と一致する fail-closed capacity、cross-page constraint、history retention を定義する。
  > 対象ファイル: `client/shared-sim/src/mvcc/`, `client/host-server/src/sim.rs`

### protocol / client

- **AOI 単位 snapshot と backpressure SLO** `0`
  > Interest enter ごとの Room 全体 snapshot を cell/lod snapshot に分割し、channel Full drop、resync 回数、subscription churn を規模 gate として記録する。
  > 対象ファイル: `protocol/src/lib.rs`, `client/app/src/bus/frame.rs`, `client/host-server/src/network_worker.rs`

- **delivery state-machine property test** `0`
  > generation、epoch、room/cell/lod tick、gap、same-tick merge を生成し、near 優先・remove wins・snapshot convergence を状態機械で検証する。
  > 対象ファイル: `protocol/src/`, `client/app/src/bus/frame.rs`

## 配布・運用

### 横断評価層

- **署名付き Windows bundle と SBOM/provenance** `0`
  > tester ZIP の client/host にコード署名、SHA-256 manifest、SBOM、GitHub artifact attestation を付け、自動更新導入前に配布 trust root を固定する。
  > 対象ファイル: `.github/workflows/windows-release.yml`, `.scripts/pack-windows-zip.ps1`

- **本番 SLO dashboard と長時間 soak** `0`
  > auth/world/assets readiness、Room epoch lag、host tick p99、cell drop/resync、relay allocate reject を同一 dashboard に集約し、24時間 room rotation と handover を soak する。
  > 対象ファイル: `client/host-server/src/metrics.rs`, `world-server/lib/world/`, `relay-agent/src/metrics.rs`
