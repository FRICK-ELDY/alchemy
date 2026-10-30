# Alchemy 第2評価者 — 具体的な強み（2026-10-30）

## 採点基準

| 点数 | 基準 |
|:---:|:---|
| +1 | 正しく実装されている |
| +2 | 一般的なベストプラクティスに沿う良い設計 |
| +3 | 同規模・同種プロジェクトの平均を明確に上回る |
| +4 | プロダクション級 OSS と比較しても遜色ない |
| +5 | 個人プロジェクトとして卓越している |

## 運営側コア

### auth-server

- **アイデンティティ・鍵・失効ライフサイクルを単一権威に集約** `+4`
  > Argon2、access/refresh、password reset/change、deactivate、RS256 `kid` / JWKS を auth-server に閉じ、他サービスは `sub` のみを参照する。Refresh rotationはDB lockと条件付き失効で競合も閉じる。
  > 対象ファイル: `auth-server/lib/auth/accounts.ex`

- **Social graph と world visibility predicate の分離** `+3`
  > Follow/FriendshipをJWTへ詰めず、authの永続正本からS2S predicateとしてworldへ提供する。unfollowとFriendship解除も同一transactionである。
  > 対象ファイル: `auth-server/lib/auth/social.ex`

- **公開・認証・内部S2S面をfail-closedで分離** `+3`
  > `/internal/*` は公網Tunnelで404、prodでは共有secret未設定で起動失敗、Social mutateにはIP+user rate limitがある。
  > 対象ファイル: `auth-server/lib/auth_web/router.ex`

### assets-server

- **物理CAS・owner reference・Grant・Quotaを段階分離した** `+3`
  > `CasObject`、owner×hashの`BlobRef`、grantee×hashの`BlobGrant`、`QuotaLedger`へ前回のglobal rowを分解した。cross-owner PUTは実bytesを再送し、各ownerのref/Grant/Quotaを作る（`blobs.ex:195-209, 279-317`）。
  > 対象ファイル: `assets-server/lib/assets/blobs.ex`

- **予約→CAS put→確定の補償境界が並行dedupeに耐える** `+2`
  > `used+reserved`を先に予約し、TX外CAS put後にrefとusedへ振り替える。失敗時に共有CASを削除せず予約だけ解放するため、同一hash並行PUTを壊さない。
  > 対象ファイル: `assets-server/lib/assets/blobs.ex`

- **local/S3切替と短命presigned GETでBEAMを大容量経路から外す** `+2`
  > immutable BLAKE3 CASをDB metadataから分け、S3時は認可後に302する。未知accessは403へfail-closedする（`blobs.ex:348-398`）。
  > 対象ファイル: `assets-server/lib/assets/cas/s3.ex`

### world-server

- **ManifestSeal oracleをcanonical DB scene照合で廃止した** `+4`
  > HostAttestationを検証し、Room ledgerのhost generationと一致させ、DB scene・protocol pin・host keyからRustのcanonical protobuf hashを再計算する。host提出revision/hashは一致確認にしか使わない（`canonical_manifest.ex:18-31`, `room_controller.ex:350-380`）。
  > 対象ファイル: `world-server/lib/world/canonical_manifest.ex`

- **host generationを永続CASで割り当てる** `+3`
  > 同一pubは世代維持、鍵変更はCASでbumpし、旧HostAttestationをseal経路で拒否する。二重host起動の競合まで明示的に扱う。
  > 対象ファイル: `world-server/lib/world/rooms.ex`

- **Room owner・7段階visibility・用途別JWTを門番で強制** `+3`
  > ownerのみhost role、social S2S障害は503、LAN/public/privateを分け、Room Token / HostAttestation / ManifestSealのaud/typを分離している。
  > 対象ファイル: `world-server/lib/world_web/controllers/room_controller.ex`

## 共通契約層

### protocol

- **ワイヤ契約とPINを独立crateのSSoTにした** `+2`
  > Manifest/Frame/Input/Presence、key helper、room/wire capability validator、role capabilityをclient外の一箇所で共有する。
  > 対象ファイル: `protocol/src/lib.rs`

- **class別pre-decode byte上限とpost-decode構造上限** `+3`
  > Manifest 1MiB、Frame 256KiB、Presence 64KiB、Input 32KiBをdecode前に拒否し、entries/upserts/strings/JWT/keyもdecode後に制限する（`decode_limits.rs:47-66, 164-210, 241-269`）。
  > 対象ファイル: `protocol/src/decode_limits.rs`

- **署名・generation/revision・配送収束を共有契約化** `+3`
  > Ed25519 canonical protobuf署名、Manifestのlexicographic watermark、PresenceのManifest世代従属、Frame gap→snapshotを共通関数と回帰テストで固定した（`generation_gate.rs:30-64`）。
  > 対象ファイル: `protocol/src/generation_gate.rs`

## ホストユーザー端末

### client/host-server

- **Token→cnf→session署名→seq→rate→finite/capabilityの多段gate** `+3`
  > bind JWTだけを盗んでも別session keyではbindできず、steady IntentはJWTを流さない。per-sub/session bucket、JWKS hard TTL、magnitude/Grab距離も強制する。
  > 対象ファイル: `client/host-server/src/session.rs`

- **physicsからcredential HTTPとZenoh waitをworker分離した** `+3`
  > 60Hz loopは最新snapshotとbounded/coalescing outbound hubだけを扱い、HTTP refreshは`credential-refresh`、encode/sign/putは`zenoh-publish`が担う。一時障害ではprocessを落とさずhard expiry後だけpublish停止する。
  > 対象ファイル: `client/host-server/src/network_worker.rs`

- **Rapier 60Hz・可変配信delta・snapshot・Presence reclaim** `+3`
  > 静止poseを省くdelta、最後にpublishしたtickのsnapshot、Leave/liveliness/TTLの同一reclaim、Manifest entry数由来のslot容量が一貫する。
  > 対象ファイル: `client/host-server/src/main.rs`

### client/app

- **偽Manifestで退室しない暗号gateとreplay防御** `+4`
  > pin/room/seal/attestation失敗はdiscardし、`(host_generation, revision)`後退と同一世代host key差替えを拒否する。PresenceもManifest世代に従属し、明示Leaveだけがlocal leaveを起こす（`main.rs:102-127, 379-485, 935-960`）。
  > 対象ファイル: `client/app/src/main.rs`

- **OpenXR swapchainをwgpu SceneGpuへ統合した** `+3`
  > XR Vulkan deviceをwgpu-halへ安全根拠付きでadoptし、dual-eye scene・UI・laser・controller overlayを同一SceneGpu経路で描く。旧ash graphics loopはtracked treeから削除済みで、Drop順序も型とコメントで固定する（`wgpu_interop.rs:164-209, 300-348`）。
  > 対象ファイル: `client/app/src/surface/xr/wgpu_interop.rs`

- **Frame真正性・有限値・tick gap・snapshot収束** `+2`
  > attested host key以外のFrameを拒否し、欠落deltaを部分適用せず署名付きsnapshotへ収束する。
  > 対象ファイル: `client/app/src/main.rs`

- **asset失敗を明示placeholderとtimeoutへ変えた** `+2`
  > releaseでは無関係なCoin fixtureへ黙ってfallbackせず、connect/read timeout、error placeholder、sticky Home statusを使う。
  > 対象ファイル: `client/app/src/main.rs`

### launcher

- **process ownershipの最小実装をCI対象にした** `+2`
  > host自身のloopback port 0 bind、one-time token IPC、ready後client起動、client-only exit、非0 crashのbounded restart、Ctrl+C shutdownを備える。
  > 対象ファイル: `launcher/src/main.rs`

## 転送・公開エッジ

### zenoh-router / edge

- **WANをmTLS・role ACL・default denyへ引き上げた** `+3`
  > guestはinput/alive、hostはmanifest/frame/presence/snapshotに限定され、cert CNをsubjectへ結ぶ。`wire_cap`をkey segmentへ入れ、アプリ署名と二層で隔離する（`config.wan.json5:21-119`）。
  > 対象ファイル: `zenoh-router/config.wan.json5`

- **Tunnelでorigin秘匿と公開route最小化を実装** `+2`
  > auth/world/assetsだけを公開し、dev/internal/catch-allをdenyする。credentialsをrepo外に置くrunbookもある。
  > 対象ファイル: `edge/cloudflared/config.yml.example`

## 横断品質

### テスト戦略

- **Hermetic Compose縦経路と否定系trafficを継続gate化** `+3`
  > login→Token→signed Intent→host→Frame/snapshotに加え、fake/stale/oversize traffic後のhost生存を確認する。
  > 対象ファイル: `.github/workflows/e2e-compose.yml`

- **境界単体テストとfuzz smokeが増えた** `+2`
  > 今回protocol 39 tests、host-server 53 testsを含むRust workspaceが成功した。protobuf checked decodeとRoom Token verifyのcargo-fuzz jobもある。
  > 対象ファイル: `.github/workflows/host-fuzz-ci.yml`

- **prod compose・test DB隔離・launcherを回帰対象にした** `+2`
  > S2S envのconfig smoke、testで`WORLD_DATABASE_PATH`無視、launcher独立jobにより前回の運用継ぎ目を自動検出する。
  > 対象ファイル: `.scripts/ci.sh`

### 可観測性・デバッグ容易性

- **秘密を出さないloopback metricsの最小面** `+2`
  > host tick lateness、Frame age、JWKS age/hard stale、CAS latency/quota rejectを固定低cardinality名で公開し、非loopback bindを拒否する。
  > 対象ファイル: `client/host-server/src/metrics.rs`

### エラーハンドリング戦略

- **一時障害・認可失敗・hard expiryを別状態にした** `+3`
  > S2S/storage失敗を404や許可へ潰さず、credential refreshはlast-goodを保持し、期限切れ後だけpublishを止める。network queueは最新値coalesceでphysicsを止めない。
  > 対象ファイル: `client/host-server/src/room_token.rs`

### 変更容易性・保守性

- **定義・実行・表示の重複知識をprotocol/shared-simへ寄せた** `+3`
  > scene分類、slot帯、interaction定数、generation gate、decode limitがhost/clientで別実装になっていない。unsafe blockはworkspace lintで根拠必須である。
  > 対象ファイル: `client/Cargo.toml`

### 開発者体験

- **Windows nativeとDocker component gateを両立** `+2`
  > Rustはnative、ElixirはDocker、prod composeはconfig smokeという現実的な導線があり、Windowsでもprotocol/client/launcher/relayの検証を実行できた。
  > 対象ファイル: `Makefile`

### セキュリティ・配布可能性

- **依存監査とWindows tester artifactを自動化** `+2`
  > cargo-deny / hex.auditをgateにし、launcher/client/host同梱ZIPをpre-releaseとして作る。
  > 対象ファイル: `.github/workflows/rust-ci.yml`

- **脅威モデルを実装可能な保証単位へ落とした** `+2`
  > trust-boundaryとassets threat modelが、攻撃手口、現状防御、目標、防御しない範囲を分けており、暗号primitiveの有無ではなく誰が何を保証するかを追える。
  > 対象ファイル: `.workspace/0_doc/architecture/trust-boundary.md`

### プロジェクト全体設計

- **短い改善sliceで前回のP0/P1を実コードまで閉じた** `+3`
  > x0038〜x0052でdiscard、canonical seal、anti-replay、decode limits、worker、cnf、rate/hard stale、mTLS ACL、metrics、fuzz、wgpu統合まで完了した。planned inventory ladderは実装済みと混同せず保証外に残している。
  > 対象ファイル: `.workspace/roadmap/README.md`

**強み合計: +86**
