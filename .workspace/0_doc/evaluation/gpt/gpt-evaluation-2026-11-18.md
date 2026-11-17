# Alchemy 第2評価者 — 総合評価（2026-11-18）

## 評価情報

### 対象

- **評価時点** `0`
  > 評価日: 2026-11-18。Git HEAD: `57ca4523cc5107772c0629fa1e7449999ab07aa3`（x00d6）。前回対象 `8833821` から約135コミット。対象は auth/assets/world、protocol、client/host-server、client/shared-sim、client/app、HostSupervisor、relay-server/agent、edge、shared/alchemy_*。launcher は廃止済み、billing/market は未設置のため対象外。
  > 対象ファイル: `.git`, `.workspace/0_doc/architecture/`

- **評価境界** `0`
  > architecture/vision/前回まとめ・自系統弱点を起点に、権限、Token/PoP、CAS、takeover、HostSupervisor、XPBD/MVCC、Spatial Interest、VCS、protocol pin を現行コードで再検証した。第1評価者の当日文書および `.workspace/0_doc/evaluation/opus/` は参照していない。目標のみの機能と proprietary 検疫ロジックは減点していない。

## 総合スコア

### 採点

| 区分 | 合計 |
|:---|---:|
| プラス | **+89** |
| マイナス | **-6** |
| 提案 | **0** |
| **正味** | **+83** |

- **総評** `+4`
  > 前回の takeover 過大権限、通常 UI host の資格情報寿命、handover 二段階、SceneBudget、refresh secret、KeyStale、UI 一枚岩はコードと否定系で閉じた。さらに MVCC×XPBD、sim_state、確定編集、Spatial Interest、Spatial VCS が実装へ進み、個人プロジェクトとして保証密度と自己改善速度は卓越している。ただし social revoke の「closed」は誤りで、通常 unfollow、推移関係、配送失敗では旧 Room 鍵が残る。物理 hot path も page allocation の閉鎖を host 全体 zero-allocation と読み替えてはいけない。

## auth-server

### ✅ プラス点

- **アイデンティティ SSoT と session lifecycle** `+5`
  > User、Argon2、RS256/kid/JWKS、access revocation、回転 refresh family、password reset/deactivate を一サービスに閉じる。
  > 対象ファイル: `auth-server/lib/auth/accounts.ex`, `auth-server/lib/auth/token/keys.ex`

- **HTTP 境界と rate 軸** `+3`
  > 公開/認証/internal を分け、IP/user/token family ごとの throttle と enumeration 抑止を持つ。
  > 対象ファイル: `auth-server/lib/auth_web/router.ex`, `auth-server/lib/auth_web/plugs/rate_limit.ex`

### ❌ マイナス点

- **social revoke 通知の発火条件と耐久性** `-2`
  > `unfollow` は friendship がない通常 follower では world 通知を送らず、送信も retry/outbox のない fire-and-forget である。world 側の推移関係問題と合わせた総減点 `-3` のうち本節へ `-2` を計上する。
  > 対象ファイル: `auth-server/lib/auth/social.ex:41-67`, `auth-server/lib/auth/world_notify.ex:1-91`

### 💡 提案

- **durable relation outbox** `0`
  > social DB mutation と event 永続化を同一境界にし、world が idempotent consume する。

**小計: +8 / -2 = +6点**

## assets-server

### ✅ プラス点

- **CAS・Quota・Grant の責務分離** `+5`
  > 物理 bytes、容量所有、読取 capability、予約補償、orphan GC を別台帳/保証単位にする。
  > 対象ファイル: `assets-server/lib/assets/blobs.ex`, `assets-server/lib/assets/cas/`, `assets-server/lib/assets/quota/`

- **Spatial VCS accept path** `+4`
  > 決定的 object verify、到達可能 graph、Grant/CAS 再検査、ref CAS/protection、Origin Cert v2 を transaction に束ねる。
  > 対象ファイル: `assets-server/lib/assets/vcs/push.ex`, `assets-server/lib/assets/vcs/cert.ex`

- **SpawnTicket ownership binding** `+3`
  > Grant-backed短命 ticketをworldの participant/room/hash/一回消費へ接続する。
  > 対象ファイル: `assets-server/lib/assets/spawn_ticket.ex`, `world-server/lib/world/spawn_ticket.ex`

### ❌ マイナス点

- **CI build volume の反復 flake** `-1`
  > 既存 CI named volume で test support module の再定義 warningが発生し、`mix compile --warnings-as-errors` が失敗した。`mix test` 240件はPASS。
  > 対象ファイル: `.scripts/ci.sh:230-249`

### 💡 提案

- **Origin Cert key history / clone-fetch** `0`
  > v2 envelope を historical key set・失効・federated fetchへ拡張する。

**小計: +12 / -1 = +11点**

## world-server

### ✅ プラス点

- **入室・実行・定義権限の分離** `+5`
  > eligible takeover、owner/editor canonical、active-host overlay、二段階 handover、lease CAS を実装する。
  > 対象ファイル: `world-server/lib/world/rooms/lease.ex`, `world-server/lib/world/rooms/scene.ex`

- **PoP・Room key・Scene 整合性** `+4`
  > nonce付き HTTP PoP、epoch current/prev、packed CAS、SceneBudget、SpawnTicket claim を重ねる。
  > 対象ファイル: `world-server/lib/world/http_pop.ex`, `world-server/lib/world/rooms/room_key.ex`, `world-server/lib/world/rooms/scene.ex`

- **Scene の原子性と資源上限** `+4`
  > canonical/overlay の packed revision CAS、409分離、409時だけのticket claim解放、Save折込、4096 entries/1MiB上限を定義境界で保証する。
  > 対象ファイル: `world-server/lib/world/rooms/scene.ex`, `world-server/lib/world_web/controllers/room_controller.ex`

### ❌ マイナス点

- **推移 social ACL の revoke 対象を列挙できていない** `-1`
  > `rotate_for_relation_change/2` は通知二者が owner の Room だけを対象にする。第三者 owner の friends_plus/followers_plus から viewer が外れた場合は回転しない。auth側 `-2` と合わせ総計 `-3`。
  > 対象ファイル: `world-server/lib/world/rooms/room_key.ex:50-89`

### 💡 提案

- **relation revision による再認可** `0`
  > 推移グラフ変更時は二者 owner 固定走査ではなく、影響 owner/relation revision を再評価する。

**小計: +13 / -1 = +12点**

## protocol

### ✅ プラス点

- **ワイヤ・暗号・世代・配送契約の SSoT** `+5`
  > PIN、decode ceiling、AEAD、session署名、generation gate、tick decisionを独立crateに置く。
  > 対象ファイル: `protocol/proto/`, `protocol/src/`

- **汎用 edit/sim_state/interest 語彙** `+3`
  > 固有コンテンツを下位層へ漏らさず、編集、handover、cell/lod deliveryを拡張する。
  > 対象ファイル: `protocol/proto/render_frame.proto`, `protocol/proto/input_events.proto`

### ❌ マイナス点

- **今回の検証境界で減点なし** `0`
  > audio/tracking/本格圧縮は未実装目標であり対象外。

### 💡 提案

- **delivery state-machine property test** `0`
  > epoch/generation/room-cell-lod tick の組合せを生成して収束を検証する。

**小計: +8 / 0 = +8点**

## client/shared-sim

### ✅ プラス点

- **Safe Rust SoA / MVCC page / XPBD** `+4`
  > aligned SoA、固定pool、CoW history、有限値 quarantine、接触、replay/page allocation回帰を持つ。
  > 対象ファイル: `client/shared-sim/src/mvcc/`

- **構造変更と物理実行の分離** `+3`
  > FrameBridge、pointer-free topology、SlotGraph、dirty codecを汎用データとして構成する。
  > 対象ファイル: `client/shared-sim/src/bridge.rs`, `client/shared-sim/src/topology.rs`

### ❌ マイナス点

- **既定 host の全 sheet clone 継続** `-2`
  > page alloc上限は閉じたが、既定Epoch経路とhost snapshotはtickごとの全sheet cloneを残す。
  > 対象ファイル: `client/host-server/src/sim_backend.rs:1-50`, `client/host-server/src/sim.rs:1220-1274`

### 💡 提案

- **allocator 計測付き実 host benchmark** `0`
  > worker/publish/Grab/接触込みのp95とallocationをdefault切替gateにする。

**小計: +7 / -2 = +5点**

## client/host-server

### ✅ プラス点

- **Untrusted input の多層 gate** `+5`
  > JWT/cnf/session/seq/capability/finite/Grab/rate/liveliness/TTLを物理前に検証する。
  > 対象ファイル: `client/host-server/src/session.rs`, `client/host-server/src/intent_rate.rs`

- **tick・publish・credential・handover 分離** `+4`
  > 60Hz/30Hz、credential worker、sim_state、epoch/MVCC接触を別単位にする。
  > 対象ファイル: `client/host-server/src/host_loop.rs`, `client/host-server/src/network_worker.rs`, `client/host-server/src/sim_state.rs`

### ❌ マイナス点

- **hot path 減点は shared-sim 節へ計上** `0`
  > 重複採点しない。

### 💡 提案

- **複数 page 化** `0`
  > SceneBudget 4096と単一page 1024の差をdirectory/cell pageへ展開する。

**小計: +9 / 0 = +9点**

## client/app / HostSupervisor

### ✅ プラス点

- **trust gate と GLB decode budget** `+5`
  > seal/attestation/signature/epoch/tick/finiteと入力・展開・画像予算を描画前に検査する。
  > 対象ファイル: `client/trust/src/`, `client/app/src/asset_decode_budget.rs`

- **Spatial Interest・LOD・予測補間** `+4`
  > per-key Moore購読、near優先、enter snapshot、exit prune、same-tick merge、jitter/predictionをendpointに閉じる。
  > 対象ファイル: `client/app/src/bus/frame.rs`, `client/app/src/epoch_mirror.rs`

- **HostSupervisor broker・OS vault・UI flow 分割** `+4`
  > refreshを子へ渡さずAUTH_PUSHし、prefsにはvault keyだけを保存し、UiActionとDesktop実行を分ける。
  > 対象ファイル: `client/app/src/host_supervisor.rs`, `client/app/src/refresh_vault.rs`, `client/app/src/surface/desktop/`

### ❌ マイナス点

- **今回の検証境界で新規減点なし** `0`
  > XR単独HTTPやDual Editor等は目標であり対象外。

### 💡 提案

- **AOI単位 snapshot** `0`
  > cell enterごとのRoom全体snapshotをcell/lod queryableへ分割する。

**小計: +13 / 0 = +13点**

## relay-server / relay-agent / edge

### ✅ プラス点

- **CSR-only短命mTLS・role ACL・AEAD** `+4`
  > forced CN、Allocation JWT、hard-stale JWKS、sub/IP throughput、default-deny非対称ACLを分ける。
  > 対象ファイル: `relay-agent/src/lib.rs`, `relay-server/config.wan.json5`

### ❌ マイナス点

- **今回の検証境界で新規減点なし** `0`
  > CCU/QoS/第三者relay経済圏は目標であり減点しない。

### 💡 提案

- **tenant/room帯域SLO** `0`
  > bytes/s、drop、subscriber churn、CCUをAllocationの次の運用gateにする。

**小計: +4 / 0 = +4点**

## 横断評価層

### ✅ プラス点

- **テスト戦略** `+5`
  > 今回 Rust workspace 708、protocol 124、relay-agent 32、auth 134、world 232、assets 240 testsが0失敗。fuzzとCompose/WAN E2E定義も維持する。
  > 対象ファイル: `.github/workflows/`

- **CI・配布・プラットフォーム gate** `+3`
  > cargo deny、Android check、Windows ZIP、prod compose、共有Elixir packagesをCIへ収録する。
  > 対象ファイル: `.scripts/ci.sh`, `.github/workflows/rust-ci.yml`, `.github/workflows/windows-release.yml`

- **評価駆動の自己改善** `+4`
  > 前回の主要欠陥を小sliceと否定系で閉じ、architectureの保証/非保証も追従させる。
  > 対象ファイル: `.workspace/roadmap/slices/x009d-host-delegation-scene-overlay.md` 〜 `.workspace/roadmap/slices/x00d6-rooms-fallback-split.md`

- **可観測性と失敗分類** `+3`
  > readiness、loopback metrics、storage 503、conflict 409、rate/JWKS/tick/drop信号を持つ。
  > 対象ファイル: `client/host-server/src/metrics.rs`, `world-server/lib/world_web/controllers/fallback_controller.ex`

### ❌ マイナス点

- **assets CI flake は assets 節へ計上** `0`
  > 重複採点しない。

### 💡 提案

- **署名bundle・SBOM・SLO soak** `0`
  > Windows artifact attestationと24時間epoch/handover/interest soakを追加する。

**小計: +15 / 0 = +15点**

## 前回指摘の再判定

### 回帰A — 自系統 2026-11-12 weaknesses

| 前回項目 | 判定 | 現行再読根拠 |
|:---|:---:|:---|
| 可視参加者の無委譲 takeover `-4` | **closed** | `eligible_host_subs`、owner/editor canonical、overlay分離（x009d/x009e） |
| Room鍵 revoke即時連携 `-2` | **partial** | direct accepted friendshipは回転。ただし通常unfollow、第三者ownerの推移関係、通知失敗が残る |
| Scene構造上限なし `-2` | **closed** | 4096 entries/1MiB SceneBudget（x00a2） |
| 60Hz全sheet clone/O(n²) `-3` | **partial** | 接触/LBVH/page poolは改善。既定hostのsheet/snapshot cloneは残る |
| UI host資格情報更新不能 `-3` | **closed** | ParentBroker + IPC AUTH_PUSH（x009f） |
| handoverがPresence先頭へ即lease `-2` | **closed** | candidate記録→accept CAS、pending中steal拒否（x00a1） |
| refresh token平文JSON `-2` | **closed** | OS keyring、prefsは`refresh_vault_key`のみ、不能時session-only（x00cf） |
| epoch前進が手動Rejoinのみ `-1` | **closed** | pending AEAD dual-open + Manifest commit、XR Rejoin委譲（x00d5） |

- **回帰件数** `0`
  > **closed 6 / open 0 / partial 2 / superseded 0**。improvement-plan の closed 注記はそのまま採用せず、現行コードで再判定した。

## ローカル検証結果

### 実行結果

| 対象 | 結果 |
|:---|:---|
| client `cargo test --workspace` | **PASS** — 708 tests, 0 failed |
| protocol `cargo test` | **PASS** — 124 tests, 0 failed |
| relay-agent `cargo test` | **PASS** — 32 tests, 0 failed |
| auth Docker checks | `mix hex.audit` は registry timeout。audit除外の format/compile/credo/migrate/test は **PASS** — 134 tests |
| world Docker checks | 初回 protocol consolidation の一時 filesystem error。再実行で format/compile/credo/test **PASS** — 232 tests |
| assets Docker checks | `mix hex.audit` は registry timeout。format/compile strict は persistent `_build` の module redefine warningで **FAIL**。`mix test`単独は **PASS** — 240 tests |
| Docker | Server 29.8.2 |
| Compose E2E / WAN実機 / OpenXR実機 | 今回未実行。workflowと受理コードを静的確認 |

- **検証の扱い** `0`
  > 全テスト本体は0失敗。Hex registry timeoutは環境要因として減点しない。assetsのstrict compile再現はローカルCI反復性の実害として `-1`。world初回filesystem errorは再実行で消えたため記録のみ。

## 結論

### 優先順位

- **P1: social revokeをdurableかつ推移関係対応にする** `0`
  > 通常unfollowを必ず通知し、第三者ownerへの影響をrelation revisionで再評価する。outbox/retry/冪等性を含める。

- **P1: 実host hot pathのcloneを除く** `0`
  > page allocationだけでclosedにせず、既定backendの全sheet/snapshot cloneと実allocation/p95を閉じる。

- **P2: assets local CI volumeを反復可能にする** `0`
  > stale test-support beamがwarnings-as-errorsを壊さないbuild pathへ変更する。

**総合: +89 / -6 = +83**
