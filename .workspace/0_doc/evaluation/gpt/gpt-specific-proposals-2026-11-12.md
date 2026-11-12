# Alchemy 第2評価者 — 具体的な提案（2026-11-12）

## 採点基準

| 点数 | 基準 |
|:---:|:---|
| 0 | 現時点では存在しないが、実装すればプロジェクトの価値を高める提案 |

## 運営側コア

### world-server

- **HostDelegation capability を入室権から分離する** `0`
  > ownerが候補sub・room・期限・権限（simulate / scene mutate）を署名し、takeoverは capability + candidate opt-in + dormant CASを必須にする。Matrix power levelやKubernetes Leaseのように「見える」「実行できる」「定義を書ける」を別権限にすると、スマートホスト移譲の土台になる。
  > 対象ファイル: `world-server/lib/world/rooms.ex`, `world-server/lib/world_web/controllers/room_controller.ex`

- **SceneBudget を protocol decode 上限と共有する** `0`
  > entries数、canonical JSON byte、mesh数、hash数を world の書込前に検査し、clientが必ずdecode可能な範囲へ固定する。manifest seal時の再検査も入れれば、既存DB汚染をfail-closedにできる。
  > 対象ファイル: `world-server/lib/world/rooms.ex`, `protocol/src/decode_limits.rs`

- **関係解除・kick と Room key epoch をイベント連携する** `0`
  > auth relation revisionまたは明示membership ledgerをworldが購読し、revoke時にepoch CASを起動する。定期24時間は鍵衛生、関係解除は認可revokeとして別SLOを持たせる。
  > 対象ファイル: `auth-server/lib/auth/social.ex`, `world-server/lib/world/rooms.ex`

## ホストユーザー端末

### HostSupervisor / client

- **親clientを短命資格情報brokerにする** `0`
  > refresh tokenは親のOS vaultだけに置き、localhost IPCでhost専用の短命access capabilityを更新配布する。子はpassword/refresh tokenを知らず、親Exitでbrokerとhostを同時に失効できる。
  > 対象ファイル: `client/app/src/host_supervisor.rs`, `client/host-server/src/host_ipc.rs`

- **Room epochを無停止で二相切替する** `0`
  > clientが旧鍵で受信中に新Token/鍵を準備し、hostの新epoch Manifestを検証後にbus generationとAEADを原子的にcommitする。grace内失敗時だけKeyStale/Rejoinへ落とす。
  > 対象ファイル: `client/app/src/scene.rs`, `client/app/src/room.rs`, `client/host-server/src/room_aead.rs`

- **OS credential vaultを共通抽象にする** `0`
  > Windows Credential Manager/DPAPI、macOS Keychain、Linux Secret Serviceをtraitで包み、prefsはkey idだけ保持する。利用不能環境はsession-onlyを明示選択させる。
  > 対象ファイル: `client/app/src/operator_prefs.rs`

- **inventory mutationにackと複合transactionを追加する** `0`
  > spawn/despawn/put-backへoperation idとhost ackを追加し、Prefab多子の部分成功を再試行可能にする。world scene CASとassets put-backを完全な分散TXにせず、saga + idempotency keyで補償するのが現実的。
  > 対象ファイル: `client/app/src/inventory.rs`, `client/host-server/src/inventory_spawn.rs`, `client/host-server/src/inventory_despawn.rs`

## 共通契約・シミュレーション

### shared-sim

- **zero-allocation tickと規模別benchmarkを品質ゲートにする** `0`
  > Prev/Curr buffer swap、scratch常駐、dirty page公開へ移行し、1k/10k/100k entityでalloc回数、p50/p95 tick、snapshot copy byteを計測する。Criterionまたは専用bench binaryをCIの非blocking記録から始め、閾値が安定したら回帰gateにする。
  > 対象ファイル: `client/shared-sim/src/bridge.rs`, `client/shared-sim/src/xpbd.rs`, `client/host-server/src/sim.rs`

- **Frame補間・jitter bufferをEpoch Prev/Currへ接続する** `0`
  > 現在のArcSwap facesとdisplay TRSを活かし、host time / arrival timeから補間alphaを算出する。欠落時は既存snapshotへ戻し、予測はuser_rootに限定すると信頼境界を崩さず体感を上げられる。
  > 対象ファイル: `client/app/src/epoch_mirror.rs`, `client/app/src/epoch_view.rs`

## 転送・運用

### relay / observability

- **relay帯域QoSとSLO集約を追加する** `0`
  > Allocation発行rateだけでなく、tenant/room別CCU・byte/s・dropをrelay側で計測し、world/clientのJWKS age、seal latency、frame ageと同じdashboardへ集約する。現状metricsは部品単位に良いので、alert ruleとrunbookが次の価値になる。
  > 対象ファイル: `relay-server/`, `relay-agent/src/metrics.rs`, `world-server/lib/world_web/telemetry.ex`

### 配布・プラットフォーム

- **署名付きinstallerと自動更新境界を設計する** `0`
  > Windows tester ZIPから、署名済みclient/hostの同一version bundle、rollback、protocol pin互換検査へ進める。host child差し替え時は親がhash/versionを検証してからspawnすると、HostSupervisorの配布保証が完成する。
  > 対象ファイル: `.scripts/pack-windows-zip.ps1`, `.github/workflows/windows-release.yml`

- **Optional Profileは実装時まで評価対象外を維持する** `0`
  > billing / market / verifier統計 / Formula / OSC / audioは現時点のコード保証ではない。追加時は既存Core Profileへ混ぜず、BSLまたは外部Proprietary注入境界と否定系contractを先に固定する。
  > 対象ファイル: `.workspace/0_doc/architecture/overview.md`, `.cursor/rules/license-and-boundaries.mdc`
