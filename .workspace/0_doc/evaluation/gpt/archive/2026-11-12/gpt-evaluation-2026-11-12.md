# Alchemy 第2評価者 — 総合評価（2026-11-12）

## 評価情報

### 対象

- **評価時点** `0`
  > 評価日: 2026-11-12。Git HEAD: `88338212e1730269b9813907ae3c88ad9359c503`（PR #187 / x009c 直後）。前回まとめ `c491016` から約122コミット。対象は auth/assets/world、protocol、client/host-server、client/shared-sim、client/app、HostSupervisor、relay-server/agent、edge、shared/alchemy_*。
  > 対象ファイル: `.git`, `.workspace/roadmap/slices/`

- **評価境界** `0`
  > architecture の保証表と現行コードを直接突き合わせた。旧 Elixir 権威実行は前提にせず、目標のみの billing/market、verifier統計、Dual-AST Kernel、OSC、audio等は減点していない。第1評価者の文書および `.workspace/0_doc/evaluation/opus/` 配下は参照していない。

## 総合スコア

### 採点

| 区分 | 合計 |
|:---|---:|
| プラス | **+82** |
| マイナス | **-19** |
| 提案 | **0** |
| **正味** | **+63** |

- **総評** `+3`
  > 前回のPoP欠如、scene更新欠落、秘密鍵配送、host rate、GLB OOM、assets flake、WAN実host不在は、ほぼすべてコードと否定系で閉じた。さらにshared-simのSoA/XPBD、HostSupervisor、host session ledgerまで進み、個人プロジェクトとして実装速度と保証密度は卓越している。一方、今回導入したhost移譲面に「可視public Roomなら誰でもdormant takeover可能」という新しい過大権限があり、通常UIログインで起動したhostが約15〜20分で資格情報更新不能になる。総合は前回 +55 から +63 へ上昇したが、新機能の制御面を過大加点しない。

## auth-server

### ✅ プラス点

- **狭いアイデンティティ SSoT** `+5`
  > User/JWT/JWKS/refresh/revocationをauthに閉じ、RoomやCAS権限を持ち込まない。鍵fail-closedと公開境界も安定している。
  > 対象ファイル: `auth-server/lib/auth/`, `auth-server/lib/auth_web/`

- **Social・S2S・readinessの分離** `+2`
  > Follow/Friendship、internal access、Repo+Keys readinessが別責務で実装される。
  > 対象ファイル: `auth-server/lib/auth/social.ex`, `auth-server/lib/auth/ready.ex`

### ❌ マイナス点

- **今回の検証境界で新規減点なし** `0`
  > メール/OAuth/Groupは目標または提案であり減点しない。

### 💡 提案

- **外部IdP導入時もUser SSoTを維持** `0`
  > IdP subjectを別tableへ写像し、auth JWT `sub` の意味を変えない。

**小計: +7 / 0 = +7点**

## assets-server

### ✅ プラス点

- **CAS・Quota・Grant/PlaceRead分離** `+5`
  > 物理bytes、Quota所有、読取権を別台帳にし、同一hash競合と補償を安全側に扱う。
  > 対象ファイル: `assets-server/lib/assets/blobs.ex`, `assets-server/lib/assets/cas/`

- **content-bound Origin Cert** `+3`
  > tree hashとparentをversioned payloadでEd25519署名し、改変・対象消失をtamperedへ倒す。
  > 対象ファイル: `assets-server/lib/assets/vcs/cert.ex`

- **hermetic test・Port pool・readiness** `+2`
  > 前回フレーク源をtest lifecycleから外し、BLAKE3常駐Portを分割・監視する。
  > 対象ファイル: `assets-server/config/test.exs`, `assets-server/lib/assets/blake3/worker.ex`

### ❌ マイナス点

- **今回の検証境界で新規減点なし** `0`
  > pgvector検疫等はライセンス境界上の外部注入対象であり減点しない。

### 💡 提案

- **Origin Certのoffline export/revocation schema** `0`
  > federation実装前にkey id、失効、再発行をversioned envelopeへ固定する。

**小計: +10 / 0 = +10点**

## world-server

### ✅ プラス点

- **host HTTP PoPとactive-host ledger gate** `+5`
  > Token、session鍵、method/path/body、active_host、leaseを重ね、bearer単独の乗っ取りを閉じた。
  > 対象ファイル: `world-server/lib/world/http_pop.ex`, `world-server/lib/world/rooms.ex`

- **Scene CASとhost操作別rate limit** `+3`
  > concurrent writerを409へ分離し、IP+jti軸でseal/scene/PlaceReadを抑える。
  > 対象ファイル: `world-server/lib/world/rooms.ex:188-299`, `world-server/lib/world_web/plugs/rate_limit.ex`

- **Room key epochとhost lease CAS** `+3`
  > visibility縮小・定期・owner手動ローテとhost-only旧鍵grace、owner/compute-host分離を永続化した。
  > 対象ファイル: `world-server/lib/world/rooms.ex:499-550,787-1089,1399-1617`

### ❌ マイナス点

- **可視参加者の無委譲 takeover** `-4`
  > public dormant Roomを非owner guestがactive_hostにでき、その後canonical scene mutation権を得る。入室権と定義・実行権が混在する。
  > 対象ファイル: `world-server/lib/world_web/controllers/room_controller.ex:569-591`, `world-server/test/world_web/controllers/room_controller_test.exs:1842-1863`

- **関係解除の即時key revoke未完** `-2`
  > epochは実装済みだがfriendship changeは回転せず、旧参加者の認可が最長約24時間残る。
  > 対象ファイル: `world-server/lib/world/rooms.ex:20-24,1540-1617`

- **Scene構造上限なし** `-2`
  > entries/JSON bytes上限がなく、長期的な正本肥大をrate limitだけでは止めない。
  > 対象ファイル: `world-server/lib/world/rooms.ex:188-374`

### 💡 提案

- **HostDelegationとSceneBudget** `0`
  > owner委譲capability、candidate opt-in、scene count/byte上限をworldの書込条件にする。

**小計: +11 / -8 = +3点**

## protocol

### ✅ プラス点

- **ワイヤ・暗号・制御面SSoT** `+5`
  > proto、PIN、decode上限、AEAD AAD、session署名、HTTP PoP、generation gateを独立crateに集約する。
  > 対象ファイル: `protocol/proto/`, `protocol/src/`

### ❌ マイナス点

- **今回の検証境界で新規減点なし** `0`
  > audio/tracking/本格圧縮は目標のため対象外。

### 💡 提案

- **delivery classのproperty test拡張** `0`
  > generation/epoch/tickの組合せを状態機械で生成し、discard/resyncを検証する。

**小計: +5 / 0 = +5点**

## client/shared-sim

### ✅ プラス点

- **汎用SoA Epoch・XPBD・pointer-free topology** `+4`
  > 固有コンテンツを持たず、世代row、readiness/dirty summary、Pod layout、間接階層、ArcSwap公開を一貫させる。
  > 対象ファイル: `client/shared-sim/src/`

### ❌ マイナス点

- **60Hzで全sheet cloneと一時allocation** `-3`
  > host実経路は1 tickで複数のEpochSheet cloneを行い、XPBD/LBVHもscratchを完全再利用していない。LBVH leaf AABB解決は線形探索で構築がO(n²)になる。
  > 対象ファイル: `client/host-server/src/sim.rs:600-634`, `client/shared-sim/src/bridge.rs:120-139`, `client/shared-sim/src/spatial.rs:169-198`

### 💡 提案

- **zero-allocation tick benchmark** `0`
  > buffer swap・常駐scratchへ変え、1k/10k entityのalloc countとp95 tickをCI記録する。

**小計: +4 / -3 = +1点**

## client/host-server

### ✅ プラス点

- **untrusted inputの多層gate** `+5`
  > JWT/cnf/session seq/finite/Grab/rate/liveliness/TTLを物理前に検査する。
  > 対象ファイル: `client/host-server/src/session.rs`, `client/host-server/src/intent_rate.rs`

- **HostLoop・network・credential worker分離** `+3`
  > 60Hz物理、30Hz配信、HTTP、Zenoh putを分け、hard expiryをtick側でも評価する。
  > 対象ファイル: `client/host-server/src/host_loop.rs`, `client/host-server/src/network_worker.rs`

### ❌ マイナス点

- **この節の減点はHostSupervisor資格情報へ計上** `0`
  > host本体のrefresh chainはrefresh tokenが渡れば機能するため、親子境界の欠陥として別節で採点する。

### 💡 提案

- **host tick allocationのprofileを公開** `0`
  > shared-sim benchmarkと同じentity fixtureでworker queueを含む実host p95を測る。

**小計: +8 / 0 = +8点**

## client/app

### ✅ プラス点

- **共有trust gateと実host WAN probe** `+5`
  > GUIとheadlessがseal/attestation/signature/epochの同一受理条件を使う。
  > 対象ファイル: `client/trust/src/`, `client/app/src/bus/`

- **GLB decode budget** `+3`
  > 入力、展開bytes、頂点、index、pixelを確保前後で制限する。
  > 対象ファイル: `client/app/src/asset_decode_budget.rs`, `client/app/src/mesh.rs`

- **Desktop/OpenXR hot switchとsession shell** `+4`
  > device差をsurfaceへ閉じ、Join/Host/Waiting/Live/KeyStaleを一貫したUI状態にした。
  > 対象ファイル: `client/app/src/surface/`, `client/app/src/system_ui/`

### ❌ マイナス点

- **refresh token平文保存** `-2`
  > 7日bearer secretを通常JSONへ保存し、OS vaultや0600強制を持たない。
  > 対象ファイル: `client/app/src/operator_prefs.rs:1-9,145-164,237-258`

- **定期epochで手動Rejoin、XR導線なし** `-1`
  > 24時間ローテもKeyStaleとなり、Desktop手動Rejoinを要求する。
  > 対象ファイル: `client/app/src/scene.rs:852-913`, `.workspace/0_doc/architecture/client.md:43`

### 💡 提案

- **OS vaultと無停止epoch切替** `0`
  > refresh secretをOSへ移し、旧鍵grace中に新epochを二相commitする。

**小計: +12 / -3 = +9点**

## HostSupervisor

### ✅ プラス点

- **オンデマンド子プロセス管理へ責務縮約** `+3`
  > room retarget、ready IPC、Job Object/PDEATHSIG、Exit latchとspawn raceを扱う。
  > 対象ファイル: `client/app/src/host_supervisor.rs`

### ❌ マイナス点

- **UI起動hostのauth更新不能** `-3`
  > 900秒access JWTだけを長命env token扱いで子へ渡しrefresh tokenを渡さないため、通常hostは約15〜20分で配信停止する。
  > 対象ファイル: `client/app/src/host_supervisor.rs:23-33,507-528`, `client/host-server/src/room_token/credential_chain.rs:180-208`

- **handover候補が任意Presence先頭** `-2`
  > opt-inを見ず5分leaseを付け、候補が起動しなければ他候補のtakeoverを塞ぐ。
  > 対象ファイル: `client/app/src/scene.rs:429-435`, `client/app/src/surface/desktop.rs:562-593`

### 💡 提案

- **親client credential broker + candidate accept** `0`
  > 親が短命tokenをIPC供給し、handoverは候補accept後の二段階CASにする。

**小計: +3 / -5 = -2点**

## relay-server / relay-agent / edge

### ✅ プラス点

- **CSR-only短命mTLSとbounded発行** `+4`
  > 秘密鍵返却を削除し、非loopback cleartextを拒否、sub/IP throughputをOpenSSL前に適用する。
  > 対象ファイル: `relay-agent/src/lib.rs`, `relay-agent/src/throughput.rs`, `relay-server/config.wan.json5`

### ❌ マイナス点

- **今回の検証境界で新規減点なし** `0`
  > CCU/帯域QoSは目標であり未実装減点にしない。

### 💡 提案

- **tenant/room別帯域SLO** `0`
  > Allocation rateに加えbyte/s、drop、CCUを計測する。

**小計: +4 / 0 = +4点**

## 横断評価層

### ✅ プラス点

- **テスト戦略** `+5`
  > unit/fuzz/否定系/WAN実hostがあり、今回client 447、protocol 73、relay-agent 32 testが通過した。
  > 対象ファイル: `.github/workflows/`, `client/host-server/fuzz/`

- **DX・単一CI入口** `+4`
  > Rust/Elixir/shared/prod composeを`.scripts/ci.sh`へ集約し、shared/alchemy_*も収録した。
  > 対象ファイル: `.scripts/ci.sh`

- **readiness・metrics・相関ID** `+3`
  > liveness/readinessを分離し、room_jti、rate reject、JWKS/tick/frame ageを主要境界に置く。
  > 対象ファイル: `auth-server/lib/auth/ready.ex`, `world-server/lib/world/ready.ex`, `client/host-server/src/metrics.rs`

- **architecture保証表の鮮度** `+3`
  > launcher廃止、HostSupervisor、epoch、PoP、scene CASまで現状と非保証を追従する。
  > 対象ファイル: `.workspace/0_doc/architecture/`

- **評価駆動の自己改善速度** `+3`
  > 前回の重大項目を小sliceと否定系で閉じる運用は高評価。ただし今回新設のhost移譲欠陥を差し引き、過大加点しない。
  > 対象ファイル: `.workspace/roadmap/slices/x0081-control-plane-pop.md` 〜 `.workspace/roadmap/slices/x009c-bind-global-session-pub.md`

### ❌ マイナス点

- **横断の重大欠陥は各責務節へ計上済み** `0`
  > takeover、host資格情報、hot-path allocationを重複採点しない。

### 💡 提案

- **性能回帰・SLO・署名配布を次の横断gateへ** `0`
  > tick benchmark、サービスSLO dashboard、署名付きclient/host bundleを段階導入する。

**小計: +18 / 0 = +18点**

## 前回指摘の再判定

### 回帰A — 2026-11-05まとめ採用マイナス

| 前回項目 | 判定 | 現行再読根拠 |
|:---|:---:|:---|
| host Room Token bearer + PoP欠如 `-3` | **closed** | inventory control planeはsession署名、host HTTPは`cnf.spk` PoP必須（x0081） |
| Room鍵非ローテ / 過去参加者revoke不可 `-3` | **partial** | epoch/visibility縮小/手動/定期は実装。friendship解除・kick相当の即時回転は未 |
| Scene revision CASなし `-3` | **closed** | expected_revision付きconditional UPDATE、409、競合test（x0082） |
| relay-agent `key_pem` cleartext `-2` | **closed** | CSR必須、responseに秘密鍵なし、非loopback cleartext起動拒否（x0083） |
| host-token HTTP rate limitなし `-2` | **closed** | seal/scene/PlaceReadをIP+jtiで分離（x0084） |
| GLB展開後上限なし `-2` | **closed** | input/expanded/vertex/index/pixel budget（x0085） |
| assets mix testフレーク `-1` | **closed** | reaper/GC test off、CAS tmp suite隔離（x0086） |
| Allocation throughput / WAN実host E2E未到達 `-1` | **closed** | sub/IP throughput（x0088）+ 実host Frame一往復（x0087） |
| P2軽微群 | **partial** | Port pool、ready、docs、owner cap、XR tempfile、main分割等はclosed。dash/desktop一枚岩等は残存 |

- **回帰件数** `0`
  > **closed 7 / open 0 / partial 2 / superseded 0**。`done` 注記ではなく、対象実装・否定系を再読して判定した。

## ローカル検証結果

### 実行結果

| 対象 | 結果 |
|:---|:---|
| `.scripts/ci.sh all` | **環境FAIL** — Windowsの`bash`がWSLへ解決され、`/bin/bash`不在 |
| client `cargo test --workspace` | **PASS** — 447 tests, 0 failed |
| protocol `cargo test` | **PASS** — 73 tests, 0 failed |
| relay-agent `cargo test` | **PASS** — 32 tests, 0 failed |
| auth Docker check | full checkはHex registry timeoutでaudit不能。auditを除くformat/compile/credo/migrate/testは **PASS** — 133 tests, 0 failed |
| world Docker check | 初回dependency compileで一時filesystem error → 再実行 **PASS** — 162 tests, 0 failed |
| assets Docker check | **PASS** — audit/format/compile/credo、191 tests, 0 failed |
| shared/alchemy_* Docker | **環境未完了** — Alpine package取得段階が無出力で停止し、約3分で終了 |
| WAN Compose / XR実機 | 今回未実行。workflow・probe・否定系を静的確認 |

- **実行上の扱い** `0`
  > Rust 3系統とauth/world/assetsのtest本体は通過した。auth auditのregistry timeout、shared image内package取得停止、shell不在は環境制約として記録し、コードFAILとは数えない。assetsは前回の初回大量failureを再現せず、一度で191 testsが通った。

## 結論

### 優先順位

- **P0: takeoverをHostDelegation必須にする** `0`
  > visibilityだけでcanonical host権限を渡さない。owner/candidate双方の同意と権限範囲をworld台帳へ置く。

- **P0: UI hostの資格情報brokerを実装する** `0`
  > 通常ログインhostが15〜20分で止まる現状を閉じ、refresh secretは親client/OS vaultに限定する。

- **P1: shared-sim hot pathをzero-allocation化する** `0`
  > SoA全cloneとO(n²)構築を除き、規模別benchmarkで保証する。

- **P1: Room revoke即時連携とSceneBudget** `0`
  > friendship/kickにepochを連動し、canonical sceneをclient受理上限内へ固定する。

**総合: +82 / -19 = +63**
