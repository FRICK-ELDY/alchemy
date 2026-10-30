# Alchemy プロジェクト総合評価 — 第2評価者 GPT-5.6 Sol（2026-10-30）

## 総合スコア

| 評価ブロック | プラス | マイナス | 正味 |
|:---|---:|---:|---:|
| auth-server | +10 | 0 | +10 |
| assets-server | +7 | -3 | +4 |
| world-server | +10 | 0 | +10 |
| protocol | +8 | 0 | +8 |
| client/host-server | +9 | 0 | +9 |
| client/app | +11 | 0 | +11 |
| launcher | +2 | -1 | +1 |
| zenoh-router / relay-agent / edge | +5 | -6 | -1 |
| Optional Profile | 0 | 0 | 0 |
| テスト戦略 | +7 | -3 | +4 |
| 可観測性 | +2 | -1 | +1 |
| エラーハンドリング | +3 | 0 | +3 |
| 変更容易性・保守性 | +3 | 0 | +3 |
| 開発者体験 | +2 | -1 | +1 |
| セキュリティ・配布 | +4 | 0 | +4 |
| プロジェクト全体設計 | +3 | 0 | +3 |
| **合計** | **+86** | **-15** | **+71** |

**評価対象:** `d2d9f44`。architectureの保証／非保証と現行コードを直接照合した。x0053〜x0065のplanned inventory/XR文書、verifier、Formula、billing、market等は未実装・評価対象外として減点していない。  
**総評:** 2026-10-28時点の「公開WANに出せない三大穴」— ManifestSeal oracle、署名済みreplay、無制限protobuf decode — は実コードと否定系testで閉じた。physics/network分離、cnf、rate/hard-stale、mTLS role ACL、wgpu XR統合まで進み、正味は **+29→+71**。一方、Relay Agentは新しい最重要境界なのに、証明書の実寿命、JWKS hard expiry、request resource bound、CIが未成熟である。次の焦点は機能追加ではなく、この門番の契約をコードどおり短命・boundedにすることである。

## 採点基準

| 点数 | 基準 |
|:---:|:---|
| +1〜+5 | 正しい実装から、個人プロジェクトとして卓越した実装まで |
| -1〜-5 | 軽微な改善余地から、根幹を揺るがす致命的欠陥まで |
| 0 | 未実装・評価対象外、または将来価値を高める提案 |

## 技術評価層

### auth-server — アイデンティティ SSoT

**小計: +10 / 0 = +10**

Argon2、access/refresh失効、password/account lifecycle、RS256 multi-key JWKS、Follow/Friendshipを一つの権威へ閉じた構成は維持されている。Social predicateをS2Sでworldへ渡し、public/internal routeとprod secretをfail-closedで分ける。Room Tokenや空間計算を引き取っていない点もarchitectureと一致する。

### assets-server — CAS・Quota・門番

**小計: +7 / -3 = +4**

前回のglobal BlobRefは`CasObject` / owner `BlobRef` / `BlobGrant`へ分離され、予約容量→TX外CAS put→確定の補償も実装された。`grant_required` GETは主体Grantを照合し、未知accessを403にする。ただしaccessを物理`CasObject`へ置いたため、最初のPUTが同一hashの全ownerに公開方針を強制する。GET既定のauthenticated capability自体は明示的非保証として文書化されたため、以前の「owner照合なし -4」をそのまま再掲せず、実害のあるpolicy squattingを `-3` とした。

### world-server — 案内所・門番・監査

**小計: +10 / 0 = +10**

canonical Manifest hashはDB scene、protocol pin、HostAttestation、永続host generationから再計算され、host提出hashは照合に限定された。generationはRoom row上のCASで割り当てられ、旧attestationもseal経路で拒否する。owner/visibility/role、S2S障害の503、用途別JWTも保たれる。verifierはコード未設置の目標なので対象外。

### protocol — ワイヤ契約 SSoT

**小計: +8 / 0 = +8**

独立crate、PIN、schema責務、key helper、role capabilityに加え、class別pre-decode byte gateとpost-decode count/string gateが入った。Manifest/Presenceのgeneration/revision decisionも共通化され、host/client別実装のずれを抑えている。今回39 unit testsが成功した。

### client/host-server — Untrusted計算ホスト

**小計: +9 / 0 = +9**

Room Token→`cnf.spk`→Ed25519 session→seq→capability→rate→finite/Grab clampを通す。JWKSは300秒hard stale、credential HTTPは専用worker、Zenoh encode/sign/putはcoalescing network workerへ移り、60Hz physicsが同期I/Oから外れた。一時credential障害ではlast-goodを使い、hard expiry後だけpublishを止める。前回の可用性・bind・rate弱点は閉じた。

### client/app — 体感・描画・UI endpoint

**小計: +11 / 0 = +11**

Manifest pin/room/seal/attestation失敗はleaveせずdiscardし、generation/revision後退と同一世代host key差替えを拒否する。Frameの署名/finite/tick/snapshot収束も維持する。asset失敗はreleaseでerror placeholderとなりHTTP timeoutも入った。XRはOpenXR Vulkan device/swapchainをwgpu-halへadoptし、SceneGpu dual-eyeとoverlayへ統合した。予測補間、OSC、audio等は未実装目標として減点していない。

### launcher — プロセスマネージャ

**小計: +2 / -1 = +1**

loopback one-time IPC、ready、client-only exit、bounded crash restartがCI対象になった。ただしhost restart後、既存clientのIPC envは古いままで、Exit Client+Hostが壊れる。deep link/tray/handoffは目標であり対象外。

### zenoh-router / relay-agent / edge

**小計: +5 / -6 = -1**

WAN zenohdはmTLS、role CN ACL、default denyとなり、guest/host key familyを分けた。`wire_cap`とアプリ署名も併用する。Cloudflare Tunnelのorigin秘匿も良い。一方、Relay Agentが返すTTLは300秒でも実証明書は`-days 1`で24時間有効で、JWKSはsoft stale無期限、body/thread/OpenSSL発行もboundedでない。新しい公開門番としてはここが最大の残件である。

### Optional Profile

verifier、billing、market-serverはリポジトリ未設置で評価対象外。BlobGrant台帳はCore assetsの現行範囲だけを評価した。

## 横断評価層

### テスト戦略

**小計: +7 / -3 = +4**

Rust workspace、protocol、launcher、3 Elixirサービス、prod compose configはローカルで通った。cargo-fuzz smoke、adversarial traffic、world test DB隔離も追加された。ただしadversarial Composeは実client trust consumerを起動せず、host生存を確認するだけである。またrelay-agentは0 testsでCI対象外である。

### 可観測性・デバッグ容易性

**小計: +2 / -1 = +1**

host tick lateness、client Frame age、JWKS age/hard-stale、assets CAS/quotaのloopback Prometheus面は有用で、公開Tunnelに同居させない判断も正しい。world/auth/relayとcollector/dashboard/SLOがまだ無く、分散障害の横断追跡は手動である。

### エラーハンドリング戦略

**小計: +3 / 0 = +3**

storage/S2S failureをnot-foundや許可へ潰さず、credential soft failure、hard expiry、queue coalesce、poison fatalを別状態にしている。BEAMとRustの境界でfail-closedと可用性を両立する設計は同規模平均を上回る。

### 変更容易性・保守性

**小計: +3 / 0 = +3**

protocol/shared-sim/auth-clientへ重複知識を移し、Scene分類・generation gate・decode limits・JWKSを共有した。unsafeはXR/OS境界へ限定し、undocumented unsafeをdenyする。planned ladderも保証単位で細分化されている。

### 開発者体験

**小計: +2 / -1 = +1**

Windows native Rust + Docker Elixirの導線、component CI、prod config smokeは実用的である。反面、README冒頭はx004d/TLS/次ID x0053のままで、x004e〜x0052の実装とずれる。

### セキュリティ・配布可能性

**小計: +4 / 0 = +4**

cargo-deny/hex.audit、Tunnel deny route、Room/session/host署名chain、tester ZIP pre-releaseが揃う。インストーラ、SBOM、artifact署名は将来提案であり現段階では減点しない。Relay Agentの具体的欠陥は転送ブロックで計上済み。

### プロジェクト全体設計

**小計: +3 / 0 = +3**

auth=identity、world=定義/門番、assets=CAS/権利、host=実行、client=endpoint、launcher=process ownership、zenoh=転送という責務がコードへ降りた。保証／非保証表とthreat modelの品質も高い。inventory ladderは目標のみと明記され、未実装を実績として数えていない。

## 前回からの変化（回帰）

前回GPT weaknesses、前回まとめweaknesses、improvement-planの重複を保証単位で正規化して再読した。**closedは15件**。根拠は現行コードのみで判定した。

| 前回項目 | 判定 | 現行根拠・残件 |
|:---|:---:|:---|
| 偽Manifest/Presenceで全員退室 | **closed** | `client/app/src/main.rs:102-127, 1919-1942`。検証失敗はdiscard、leaveは明示経路のみ |
| ManifestSeal任意hash oracle | **closed** | `world-server/lib/world/canonical_manifest.ex:18-31`、controller `359-380` |
| Manifest/Presence replay | **closed** | `protocol/src/generation_gate.rs:30-64`、client `403-485, 940-960` |
| protobuf decode byte/count無制限 | **closed** | `protocol/src/decode_limits.rs:47-66, 164-269` |
| prod compose S2S env欠落 | **closed** | auth/world prod composeで必須展開、prod config smoke成功 |
| world test DB汚染 | **closed** | `world-server/config/test.exs:3-9`、65 tests再実行成功 |
| assets GET所有者/Grant | **partial** | `grant_required`はGrant照合、既定authenticatedは意図的非保証。新規のglobal access policy問題へ置換 |
| CAS object/ref混同・補償なし | **closed** | `blobs.ex:195-317`。CasObject/BlobRef/Grantとreserved補償 |
| physics loop同期HTTP/Zenoh wait | **closed** | `credential-refresh` / `zenoh-publish` worker |
| bind JWT盗聴（cnfなし） | **closed** | Room Token `cnf.spk`とsession_pub一致必須 |
| Intent rate / JWKS hard staleなし | **closed** | `intent_rate.rs` token bucket、`room_token.rs:39-42, 239-244` |
| WAN mTLS / ACLなし | **partial** | mTLS+role ACL+wire_capは実装。per-room cert subject/QoSは非保証 |
| adversarial E2Eなし | **partial** | fake/stale/oversize trafficは追加。実client trust pathは未通 |
| fixture fallback / HTTP timeout | **closed（2件）** | release placeholder、connect 5s/read 30s |
| launcher CI外 / restart policy | **partial** | CIとpolicy testはclosed。client IPC staleはopen |
| metrics不在 | **closed** | host/client/assets loopback metrics。横断SLOは別の軽微残件 |
| fuzz不在 | **closed** | protobuf/Room Token cargo-fuzz smoke job |
| README鮮度 | **open** `-1` | `README.md:9-18`がx004d/TLS/次ID x0053 |
| XR描画経路二重化 | **closed** | tracked `vk_loop.rs`削除、`wgpu_session`既定 |

## ローカルCI結果

Windows PowerShell環境のため`make ci`そのものは未実行（`make`なし）。同等commandをcomponent別に実行した。Elixirのworld/assetsは最初のonline `hex.audit`がHex registry timeoutで停止したため、同一lock/cacheに対して`HEX_OFFLINE=1`で再実行し、auditを含む残りのgateが成功した。これはコード失敗ではなく外部registry到達の制約である。

| 対象 | 結果 |
|:---|:---|
| protocol `cargo test` | **成功** — 39 tests |
| protoc 4 schema compile | **成功** |
| client `cargo fmt --check` | **成功** |
| client `cargo clippy --workspace -D warnings` | **成功** |
| client `cargo test --workspace` | **成功** — host-server 53 testsを含む全crate |
| launcher fmt/clippy/test | **成功** — 4 tests |
| relay-agent fmt/clippy/test | **成功** — ただし **0 tests** |
| auth Docker gate | **成功** — audit/format/compile/credo/migrate、122 tests |
| world Docker gate | **成功（offline audit再試行）** — format/compile/credo、65 tests |
| assets Docker gate | **成功（offline audit再試行）** — format/compile/credo、34 tests |
| prod compose config smoke | **成功** — auth/world S2S env確認 |
| Compose E2E / XR実機 | 今回は未実行。定義とprobeコードを再読 |

## 同種OSS比較

Bevy/Godotと比べるとeditor、asset import、animation/audio、platform breadthはまだ技術デモ段階である。VRChat/Resonite系と比べてもUGC経済圏、handoff、moderation、実ユーザー規模の運用実績はない。一方、identity/world/host/client/relayのtrust boundaryを明文化し、Room Token→cnf session→world canonical seal→attested signed Frame→anti-replayまで実装した密度は、同規模の個人OSSを明確に上回る。

成熟したEnvoy/Linkerd系門番やZenoh運用と比べると、Relay Agentの証明書寿命、hard stale、bounded concurrency、rate/QoS、監視が不足する。renderer側はwgpu/OpenXR device adoptionとdrop順序の扱いが高度だが、実HMD/GPUの継続CIとjitter/interpolationはBevy/Godot級には未到達である。

## 次にやるべきトップ5

1. **Relay証明書の実NotAfterを300秒へ一致** `-3`
2. **Relay Agentへhard JWKS TTL・body/thread/issuance rate上限** `-3`
3. **CAS accessを物理objectからGrant/refへ移す** `-3`
4. **実client trust consumerをadversarial Compose E2Eへ追加** `-2`
5. **launcher固定discovery endpointでrestart後IPCを更新** `-1`

## 一言評価

> **前回の暗号契約P0を実装で閉じ、公開WAN候補へ大きく前進した。今はRelay Agentだけが「短命・失効可能・bounded」という全体水準に追いついていない。**
