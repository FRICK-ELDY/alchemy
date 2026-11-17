# Alchemy 第2評価者 — 具体的な強み（2026-11-18）

## 採点基準

| 点数 | 基準 |
|:---:|:---|
| +1 | 正しく実装されている |
| +2 | 業界の一般的なベストプラクティスに沿う良い設計判断 |
| +3 | 同規模・同種プロジェクトの平均を明確に上回る |
| +4 | プロダクションレベルのゲームエンジン・OSSと比較しても遜色ない |
| +5 | このクラスの個人プロジェクトでは見ない卓越した実装 |

## 運営側コア

### auth-server — アイデンティティ SSoT

- **認証ライフサイクルを狭い SSoT に閉じている** `+5`
  > Argon2、RS256/kid/JWKS、access revocation、回転 refresh family、reuse 検知、password reset、account deactivate を auth に集約し、Room/CAS 権限を JWT へ混入させない。公開エラーも account enumeration を抑えている。
  > 対象ファイル: `auth-server/lib/auth/accounts.ex`, `auth-server/lib/auth/token/keys.ex`, `auth-server/lib/auth_web/controllers/auth_controller.ex`

- **公開・認証・internal 境界と rate 軸が分離されている** `+3`
  > login/register/refresh/social を IP・user・token family 軸で抑え、prod internal token と Tunnel 404 を別境界にしている。
  > 対象ファイル: `auth-server/lib/auth_web/router.ex`, `auth-server/lib/auth_web/plugs/rate_limit.ex`

### assets-server — CAS・Spatial VCS

- **物理 CAS・Quota・読取権を別台帳に分離する** `+5`
  > CasObject は所有者を持たず、BlobRef が容量、BlobGrant/PlaceRead/RepoRead が読取権を担う。予約→TX外 put→確定と orphan quarantine は並行同一 hash を破壊しない。
  > 対象ファイル: `assets-server/lib/assets/blobs.ex`, `assets-server/lib/assets/cas/`, `assets-server/lib/assets/quota/`

- **内容アドレス Spatial VCS の push 境界が堅い** `+4`
  > 決定的 CBOR、Port verify、到達可能 commit、generation、Grant/CAS の TX 内再検査、ref CAS、protected branch、Authorship/Release v2 を一つの accept path に束ねる。
  > 対象ファイル: `assets-server/lib/assets/vcs/push.ex`, `assets-server/lib/assets/vcs/cert.ex`, `protocol/src/vcs_object.rs`

- **SpawnTicket で所有権確認と配置を結ぶ** `+3`
  > assets の Grant を短命 RS256 ticket にし、world が room/hash/participant と一回消費を検査する。host を信頼せずに配置権を移送している。
  > 対象ファイル: `assets-server/lib/assets/spawn_ticket.ex`, `world-server/lib/world/spawn_ticket.ex`

### world-server — 案内所・門番

- **見る権利・計算ホスト・定義書込を分離した** `+5`
  > takeover は owner/eligible allow-list、canonical は owner/editor、非 writer host は overlay のみである。host 交代時 overlay clear、二段階 handover accept、lease CAS が前回の過大権限を実装上閉じた。
  > 対象ファイル: `world-server/lib/world/rooms/lease.ex`, `world-server/lib/world/rooms/scene.ex`, `world-server/lib/world_web/controllers/room_controller.ex`

- **Room Token・HTTP PoP・鍵 epoch を多層化した** `+4`
  > RS256 Room Token の `cnf.spk`、method/path/body/timestamp/nonce PoP、active-host ledger、Room AEAD current/prev epoch を重ね、bearer 単独・skew 内 replay・旧鍵をそれぞれ別層で閉じる。
  > 対象ファイル: `world-server/lib/world/http_pop.ex`, `world-server/lib/world/token.ex`, `world-server/lib/world/rooms/room_key.ex`

- **Scene の原子性と資源上限を定義境界で保証する** `+4`
  > canonical/overlay の packed revision CAS、409 分離、409 時だけ SpawnTicket claim 解放、4096 entries/1MiB budget、Save の親・field・spawn 折込を world 正本で検査する。
  > 対象ファイル: `world-server/lib/world/rooms/scene.ex`, `world-server/lib/world_web/controllers/fallback_controller.ex`

## 共通契約・ホスト

### protocol

- **ワイヤ・鍵式・暗号・世代ゲートの SSoT** `+5`
  > Protobuf/PIN、f64 描画境界、decode ceiling、AEAD AAD、Ed25519 session、generation/revision、delivery tick を独立 crate に集約する。
  > 対象ファイル: `protocol/proto/`, `protocol/src/lib.rs`, `protocol/src/wire_aead.rs`, `protocol/src/generation_gate.rs`

- **編集・handover・Spatial Interest 契約を汎用語彙で拡張した** `+3`
  > EditProposal/Commit、sim_state、cell/lod key、holder/release を特定コンテンツ名なしで追加し、server=定義/client=実行を維持する。
  > 対象ファイル: `protocol/proto/render_frame.proto`, `protocol/proto/input_events.proto`, `protocol/src/lib.rs`

### client/shared-sim

- **SoA/MVCC page/XPBD の汎用土台** `+4`
  > 64-byte aligned SoA、固定 page pool、CoW history、holder private column、有限値 quarantine、接触付き PageXpbdAdapter を Safe Rust で構成する。1k entity×500 tick の page allocation 上限と replay 一致をテストする。
  > 対象ファイル: `client/shared-sim/src/mvcc/page.rs`, `client/shared-sim/src/mvcc/arena.rs`, `client/shared-sim/src/mvcc/adapter.rs`

- **確定編集と物理 topology を分離した** `+3`
  > FrameBridge、pointer-free topology、SlotGraph、dirty codec により、構造変更を tick barrier に載せ、固有 UI 知識を shared-sim に持ち込まない。
  > 対象ファイル: `client/shared-sim/src/bridge.rs`, `client/shared-sim/src/topology.rs`, `client/shared-sim/src/delta_codec.rs`

### client/host-server

- **Untrusted input の多層 gate** `+5`
  > JWT/cnf、session signature、sequence、role capability、finite、Grab 距離、rate、Leave/liveliness/TTL を物理適用前に検査する。
  > 対象ファイル: `client/host-server/src/session.rs`, `client/host-server/src/intent_rate.rs`, `client/host-server/src/finite.rs`

- **60Hz・30Hz・credential・handover を分離した** `+4`
  > 物理 loop、dirty network publish、credential worker、sim_state planned handover を別保証単位にし、接触回帰と epoch/MVCC 両 backend の軌道継続をテストする。
  > 対象ファイル: `client/host-server/src/host_loop.rs`, `client/host-server/src/network_worker.rs`, `client/host-server/src/sim_state.rs`

## クライアント・転送

### client/app / HostSupervisor

- **受信 trust gate とアセット予算が描画前にある** `+5`
  > seal/attestation/signature/epoch/tick/finite を共有 gate で検査し、GLB は入力・展開・accessor・画像寸法を確保前後で制限する。
  > 対象ファイル: `client/trust/src/`, `client/app/src/asset_decode_budget.rs`, `client/app/src/mesh.rs`

- **Spatial Interest と補間を endpoint 側へ閉じた** `+4`
  > Moore 近傍を per-key で差分購読し、near を lod より優先、enter 時 snapshot、exit prune、同 tick multi-cell merge、手元予測/jitter を client 内で処理する。client は中継しない。
  > 対象ファイル: `client/app/src/bus/frame.rs`, `client/app/src/epoch_mirror.rs`, `client/app/src/prediction.rs`

- **親子資格情報と UI 実行責務を整理した** `+4`
  > HostSupervisor は access JWT を AUTH_PUSH し refresh secret を子へ渡さない。refresh は OS vault、UI は `UiAction`、Desktop 実行は session/inventory/hotswitch flow に分割した。
  > 対象ファイル: `client/app/src/host_supervisor.rs`, `client/app/src/refresh_vault.rs`, `client/app/src/surface/desktop/`

### relay-server / relay-agent / edge

- **短命 mTLS と非対称 ACL が二重防壁になっている** `+4`
  > CSR-only 証明書、強制 role CN、Allocation JWT、hard-stale JWKS、sub/IP throughput、zenohd default-deny と Room AEAD を分離する。
  > 対象ファイル: `relay-agent/src/lib.rs`, `relay-server/config.wan.json5`, `edge/cloudflared/`

## 横断評価層

### 品質・運用

- **テスト戦略が保証境界に追従する** `+5`
  > unit/否定系/property 的回帰、cargo-fuzz、Compose/WAN host E2E を持つ。今回 Rust workspace 708、protocol 124、relay-agent 32、auth 134、world 232、assets 240 tests が全て PASS した。
  > 対象ファイル: `.github/workflows/`, `client/host-server/fuzz/`

- **CI・配布・マルチプラットフォームの実線がある** `+3`
  > 単一 local CI、cargo deny、Android shared-sim check、Windows tester ZIP、prod compose smoke を継続 gate にしている。
  > 対象ファイル: `.scripts/ci.sh`, `.github/workflows/rust-ci.yml`, `.github/workflows/windows-release.yml`

- **評価駆動の自己改善がコードと否定系まで届く** `+4`
  > 6日で前回の takeover、credential broker、handover、SceneBudget、vault、KeyStale、UI 分割を小 slice と回帰テストで閉じた。文書の closed 注記だけでなく対象実装を再読して確認できた。
  > 対象ファイル: `.workspace/roadmap/slices/x009d-host-delegation-scene-overlay.md` 〜 `.workspace/roadmap/slices/x00d6-rooms-fallback-split.md`

- **readiness・metrics・失敗分類が境界ごとにある** `+3`
  > liveness/readiness、loopback metrics、rate reject、JWKS age、tick lateness、storage 503、scene conflict 409 を別信号にする。
  > 対象ファイル: `world-server/lib/world/ready.ex`, `client/host-server/src/metrics.rs`, `world-server/lib/world_web/controllers/fallback_controller.ex`

**強み合計: +89**
