# Alchemy 第2評価者 — 具体的な提案（2026-10-30）

## 採点基準

| 点数 | 基準 |
|:---:|:---|
| 0 | 現時点では未実装・評価対象外だが、実装すれば価値を高める提案 |

## 運営側コア

### auth-server

- **鍵ローテーション演習と失効SLOを定期化する** `0`
  > JWKS multi-key実装はある。旧鍵追加→新鍵署名→旧鍵撤去をstagingで自動演習し、host/client/relayのhard-stale挙動まで観測すると運用保証が上がる。
  > 対象ファイル: `auth-server/lib/auth/token/keys.ex`

### assets-server

- **accessをGrant側へ移しGETを用途capabilityで統一する** `0`
  > 物理CASをpolicy-freeにし、owner/room/release/purchased grantへaccess scopeと期限を置く。案Bへの移行は既存room meshへ短命room grantを先に配ってからdefaultを閉じる。
  > 対象ファイル: `assets-server/lib/assets/blobs.ex`

- **予約reaper・orphan CAS GC・streaming hashを追加する** `0`
  > crash後reserved rowとDB未確定CASを回収し、大容量PUTを全bodyメモリ保持せず一度だけstream hash/putする。新規依存が必要なら人の承認後に進める。
  > 対象ファイル: `assets-server/lib/assets/blobs.ex`

### world-server

- **canonical Manifest完成bytesをworldから返す** `0`
  > 現在はworldとhostが同じbodyを別々に組み立て、hash一致で検査する。worldがseal済みprotobufを返せば「server=定義、host=実行」を型・bytes単位でさらに単純化できる。
  > 対象ファイル: `world-server/lib/world/canonical_manifest.ex`

## 共通契約・実行

### protocol / shared-sim

- **message budgetを運用値と同じ場所でversion管理する** `0`
  > decode上限に加え、router payload、per-peer byte/sec、Manifest entity costを同じ契約表から生成すると、上限の片側変更を防げる。
  > 対象ファイル: `protocol/src/decode_limits.rs`

- **Slot graph / inventory roundtripはplanned ladder順で実装する** `0`
  > x0054〜x0065は現時点で文書のみなので減点対象外。Grant-required PUT→held slot→take-out/put-back→rejoinを先に閉じ、Drive/VCS/Origin Certへ進む順序は妥当である。
  > 対象ファイル: `.workspace/roadmap/inventory-ladder.md`

### client/host-server

- **network queue depth・coalesce・publish latencyをmetrics化する** `0`
  > worker分離済みなので、queue saturationとdrop/coalesce理由を低cardinalityで出せば、60Hz維持とWAN障害を定量的に判別できる。
  > 対象ファイル: `client/host-server/src/network_worker.rs`

- **Formula VM・Wake-on-Join・handoffは現境界を維持して追加する** `0`
  > Formulaはdeterministic sandbox、Wakeはroom lifecycle、handoffはworld generation証明+snapshot transferとして実装する。未実装目標なので現評価では減点しない。

## ホストユーザー端末

### client/app

- **verified stateとrender stateの補間bufferを分離する** `0`
  > 現在の真正性gateを崩さず、2〜3 Frame interpolation、extrapolation上限、teleport thresholdを追加し、Frame ageと体感を結びつける。
  > 対象ファイル: `client/app/src/scene.rs`

- **XR↔Desktop hot switchをresource state machineとして実装する** `0`
  > x0053はplannedであり減点しない。OpenXR session喪失時にSceneGpu/shared stateを保持し、surface resourceだけを再生成する設計が適する。
  > 対象ファイル: `.workspace/roadmap/slices/x0053-xr-desktop-hotswitch.md`

- **OSC・audio/tracking・LODはFrameとは別budgetにする** `0`
  > 目標機能は未実装・評価対象外。入力edgeでfinite検証し、audio/trackingは独立message class、LODはCAS bundle manifestとして追加する。

### launcher

- **固定discovery endpointをdeep link/tray/handoffのcontrol planeにする** `0`
  > clientが安定したlauncher endpointを見てhost generation/address更新を購読すれば、stale IPC修正と将来機能を同じ境界で解ける。
  > 対象ファイル: `launcher/src/main.rs`

## 転送・公開エッジ

### relay-agent / zenoh-router

- **秒単位証明書・hard stale・bounded issuanceを一括で閉じる** `0`
  > Allocation Tokenのexp/jti、証明書NotAfter、JWKS hard TTL、per-sub/relay rate、発行worker上限を一つの受理契約にする。証明書CN/SANへroom/wire capabilityを直接載せるかはzenoh ACL hot reloadとのtrade-offを別設計する。
  > 対象ファイル: `relay-agent/src/main.rs`

- **QoS/CCUはRelay Agent前段で測定可能にする** `0`
  > 現在のrole ACLを維持し、allocation単位のconnection/byte budgetとdeny reasonを追加する。billing未実装の間も固定free-tier値で検証できる。
  > 対象ファイル: `zenoh-router/config.wan.json5`

## 横断品質

### テスト

- **headless client trust consumerをCompose E2Eへ入れる** `0`
  > signed old Manifest replay、same-generation key swap、bad seal、Presence regression、oversize sampleをappと同じ関数で受信し、joined維持とstate不変をassertする。
  > 対象ファイル: `.github/workflows/e2e-compose.yml`

- **XR interopを実GPU nightlyで煙テストする** `0`
  > compile/unitに加え、Windows self-hostedまたは専用runnerでSteamVR/OpenXR acquire→render→releaseとdevice lossを低頻度gateにする。
  > 対象ファイル: `client/app/src/surface/xr/wgpu_session.rs`

### 可観測性・配布

- **loopback collector・dashboard・SLOを追加する** `0`
  > public metrics endpointを増やさず、host/client/assets/world/auth/relayのlocalhost面をagentが収集する。room/subをlabelにせず、service/class/reasonで集約する。

- **SBOM・artifact署名・自動更新channelを追加する** `0`
  > tester ZIPまで到達した次段として、再現build、SBOM、署名、rollback可能なinstaller/update manifestを整備する。

### Optional Profile

- **verifierはcanonical Manifestとgenerationを前提にSpot-Checkから始める** `0`
  > 先行条件は整った。最初はFrameの速度/asset membershipを抜き打ち検査し、誤検知時のToken失効影響を観測する。コード未設置のため現評価では対象外。

- **billing / marketはGrantとRelay Allocationの署名chainを先に固定する** `0`
  > Optional Profileは未設置で減点しない。Webhook署名+jti冪等、License→BlobGrant、Relay plan→Allocation Tokenを境界契約から始める。

**提案合計: 0**
