# Alchemy 第2評価者 — 具体的な提案（2026-10-28）

## 採点基準

| 点数 | 基準 |
|:---:|:---|
| 0 | 現時点では未実装・評価対象外だが、実装すれば価値を高める提案 |

## セキュリティ境界

### CAS capability

- **CasObject・BlobRef・BlobGrant を分離する** `0`
  > global dedupe object、owner が持つ論理 reference、viewer/license capability を別 table にする。GET は owner/grant を照合し、presigned URL はその後に生成する。upload reservation と compensation job で CAS orphan も回収できる。
  > 対象ファイル: `assets-server/lib/assets/blobs.ex`

### world-authored Manifest

- **world が canonical Manifest を構築して直接 seal する** `0`
  > host から任意 hash を受け取らず、world DB の scene、attested host key、protocol pin から deterministic protobuf body/hash を生成する。可能なら完成済み Manifest を world が返し、host はそのまま publish する。
  > 対象ファイル: `world-server/lib/world/manifest_seal.ex`

- **host generation と revision の replay protection を契約化する** `0`
  > ManifestSeal に monotonic `scene_revision` と `host_generation`、Presence に generation を入れ、client は `(room, generation, revision)` の後退を拒否する。host handoff 時だけ world の新 generation 証明で切り替える。
  > 対象ファイル: `protocol/proto/scene_manifest.proto`

### Transport resource limits

- **message class ごとの byte/count limit を decode 前後で強制する** `0`
  > Zenoh sample bytes を decode 前に上限確認し、decode 後も entries/upserts/removes/assignments/string 長を制限する。router 側 max payload と per-peer token bucket も併用する。
  > 対象ファイル: `client/host-server/src/main.rs`

- **mTLS identity と room/tenant key ACL を Relay Agent に置く** `0`
  > transport identity→tenant→room capability を短命 allocation token で結び、publish/subscribe/queryable key を最小化する。CCU/帯域 quota は router 前段で enforcement する。
  > 対象ファイル: `zenoh-router/config.wan.json5`

## 実行・体感

### host-server

- **physics と network/credential worker を bounded channel で分離する** `0`
  > physics は immutable snapshot/delta を再利用 buffer へ書き、network worker が publish する。満杯時の drop/coalesce policy、deadline miss、queue depth を metric にする。
  > 対象ファイル: `client/host-server/src/main.rs`

- **per-session rate limit と strike policy を追加する** `0`
  > Intent Hz、bind refresh、Grab transition を `sub/session_pub` ごとの token bucket で制限し、drop reason と temporary ban を集約する。
  > 対象ファイル: `client/host-server/src/session.rs`

- **Formula VM・Wake-on-Join・handoff は次段として実装する** `0`
  > 現在の shared-sim/launcher 境界を保ち、Formula は deterministic sandbox、Wake-on-Join は room lifecycle、handoff は world generation 証明と snapshot transfer として追加する。未実装の目標機能なので現評価では減点しない。

### client/app

- **error placeholder と retryable asset state を導入する** `0`
  > fixture fallback は debug feature に限定し、本番は hash mismatch/download/auth failure を異なる placeholder と UI status で表示する。再認証後の再取得と live mesh swap を state machine にする。
  > 対象ファイル: `client/app/src/main.rs`

- **補間・jitter buffer・Frame age 表示を追加する** `0`
  > verified authoritative state と render state を分け、2–3 frame buffer、interpolation/extrapolation 上限、teleport threshold を持たせる。目標の60–120fps体感を測定可能にする。
  > 対象ファイル: `client/app/src/scene.rs`

- **OSC・Dual Editor・LOD/KTX2・audio/tracking を契約単位で追加する** `0`
  > いずれも目標のみで、現時点の減点対象ではない。audio/tracking は frame/input と別 message class、Dual Editor は単一 AST、LOD は CAS bundle manifest として追加する。

### launcher

- **再起動可能な IPC discovery channel を設ける** `0`
  > client が env の固定 address ではなく launcher の安定 endpoint へ接続し、host generation/address update を購読する。将来の tray/deep link/handoff も同じ process-control plane に載せる。
  > 対象ファイル: `launcher/src/main.rs`

## 品質・運用

### テスト

- **adversarial E2E と fuzz corpus を追加する** `0`
  > old signed Manifest/Presence replay、oversized protobuf、unknown kid rotation、R2 302/hash mismatch、Zenoh disconnect/backpressure を Compose fault test にする。protobuf/JWT/GLB は cargo-fuzz、visibility graph は StreamData/property test が適する。
  > 対象ファイル: `.github/workflows/e2e-compose.yml`

### 可観測性

- **OpenTelemetry/Prometheus と SLO dashboard を導入する** `0`
  > room/sub を高 cardinality label に直接せず、service/room class/reason を軸に tick lateness、frame age、resync、JWT refresh、CAS/S2S latency、quota/rate reject を収集する。

### 配布

- **release artifact・SBOM・署名・installer を整備する** `0`
  > launcher/client/host の再現 build、cargo auditable/SBOM、artifact signing、Windows installer/update channel を CI に置く。現状は目標段階なので減点しない。

### Optional Profile

- **verifier・billing・market-server は境界が固まってから追加する** `0`
  > verifier は world-authored Manifest と host generation が先行条件。billing/market は Relay Allocation/License Token と BlobGrant を capability chain として設計する。ディレクトリ未設置のため評価対象外。

**提案合計: 0**
