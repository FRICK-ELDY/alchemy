# Alchemy 未完了リスト（優先度付き / 2026-11-04 時点）

正本の進捗目次は [README.md](README.md)。本ファイルは **未完了帯の優先度ビュー**。

**既に閉じた前提（このリストから除外）:** Desktop 手通し〜x006a、inventory Wave 0–4（x0054–x0065）、CAS access 共存 peel、`tos_agreed` UI、**x0053 hotswitch**、**x006b PlaceRead**、**x006c 案 B**、**x006d host take-out**、**x006e put-back UI**、**x006f access 列廃止**、**x0070 trust WAN E2E**、**x0071 world RL / Room cap**、**x0072 launcher discovery**。**x0073** 軽量衛生 **done**。**x0074** reserved reaper / orphan CAS GC **done**。**x0075** BLAKE3 / seal-hash 常駐 Port **done**。**x0076**–**x0078**（保証単位分割 / bind 送信元 / mTLS temp）**done**。**x0079** DemoScene 共有テストベクタ **done**。**x007a** metrics+jti **done**。**x007b** scene despawn **done**。**x007c** Worlds カタログ / Join・Host **done**。**x007d** Live Manifest remount **done**。**x007e** Room key epoch **done**。**x007f** Prefab 多子 / XR Inventory **done**。**x0080** host handover / takeover + dormant **done**。**x0081** 制御面 PoP **done**。**x0082** Scene CAS **done**。**x0083** Agent CSR-only **done**。**x0084** host-token RL **done**。**x0085** GLB decode budget **done**。空き ID = `x0086`。

---

## P0 — いまの次（本線）

| 優先 | ID | 内容 | 状態 |
|:---|:---|:---|:---|
| **P0-閉** | [x0081](slices/x0081-control-plane-pop.md) | 制御面 session 署名 + world host HTTP PoP（improvement P0-1） | **done** |
| **P0-閉** | [x007f](slices/x007f-prefab-multi-xr-inventory.md) | Prefab 多子展開 + XR Inventory 薄皮（Live Inventory） | **done** |
| **P0-閉** | [x007c](slices/x007c-worlds-catalog-join-host.md) | Worlds カタログ / Join・Host 分離（見える・入れる・読める・回す） | **done** |
| **P0-閉** | [x007e](slices/x007e-room-key-epoch.md) | Room key epoch / revoke-driven rotation（improvement P0-2） | **done** |
| **P0-閉** | [x007d](slices/x007d-live-manifest-mesh-remount.md) | Live Manifest mesh remount（MainHub 空シェル） | **done** |
| **P0-ladder** | x0054–x0065 | inventory Wave 0–4 | **done（host take-out UI は x006d）** |
| **P0-体感** | [x0053](slices/x0053-xr-desktop-hotswitch.md) | XR ↔ Desktop ホットスイッチ | **done** |
| **P0-発行** | [x006b](slices/x006b-place-read-capability.md) | PlaceRead（場所で付与・seal 連動） | **done** |
| **P0-案B** | [x006c](slices/x006c-blob-grant-option-b.md) | BlobGrant 案 B（GET 常時 Grant / PlaceRead） | **done** |
| **P0-take-out** | [x006d](slices/x006d-host-take-out-control.md) | host take-out（制御面・Drive HTTP・Inventory UI） | **done** |
| **P0-put-back** | [x006e](slices/x006e-inventory-put-back-ui.md) | Inventory put-back UI | **done** |
| **P0-access** | [x006f](slices/x006f-cas-access-grant-peel.md) | CAS access → Grant/ref 移設（P1-1） | **done** |
| **P0-閉** | [x0073](slices/x0073-p3-hygiene-easy.md) | P3 軽量衛生（Keys PT / prod publish / launcher deny / smoke） | **done** |
| **P0-閉** | [x0074](slices/x0074-reserved-reaper-orphan-cas-gc.md) | reserved reaper / orphan CAS GC | **done** |
| **P0-閉** | [x0075](slices/x0075-blake3-seal-hash-port.md) | BLAKE3 / seal-hash 常駐 Port（P2-2） | **done** |
| **P0-閉** | [x0076](slices/x0076-guarantee-unit-split.md) | host/client 保証単位分割（P2-4） | **done** |
| **P0-閉** | [x0077](slices/x0077-bind-source-split.md) | bind 送信元分割（P2-5） | **done** |
| **P0-閉** | [x0078](slices/x0078-mtls-temp-acl-sweep.md) | mTLS 鍵 temp の権限・掃除（P2-9） | **done** |
| **P0-閉** | [x0079](slices/x0079-demoscene-shared-vector.md) | DemoScene 共有テストベクタ（P2-11） | **done** |
| **P0-閉** | [x007a](slices/x007a-metrics-jti-correlation.md) | auth/world/relay metrics + `jti` 相関（P2-12） | **done** |
| **P0-閉** | [x007b](slices/x007b-scene-despawn.md) | scene despawn（world DELETE / host / Inventory） | **done** |
| **P0-閉** | [x0072](slices/x0072-launcher-discovery.md) | launcher discovery（P1-5） | **done** |
| **P0-閉** | [x0071](slices/x0071-world-rate-limit-room-cap.md) | world RL + Room per-owner cap（P1-4） | **done** |
| **P0-隣接** | verifier | Spot-Check | planned / 保留 |

---

## P1 — 門番・権利の残り穴（評価残 / 機密性の前提）

10/30 改善計画のうち、x0066〜x0069 で閉じた relay/AEAD 以外の残件。inventory が「私有」を本気で載せる前に閉じたい帯。

| 優先 | 項目 | 根拠 |
|:---|:---|:---|
| **P1-1** | **CAS access を Grant/ref へ移設**（先着固定解消） | **→ x006f done** |
| **P1-2** | **BlobGrant 案 B**（GET 常時 Grant + room 連動読取） | **→ x006c done** |
| **P1-3** | **headless client trust + WAN 否定系 E2E** | improvement P1-2。**→ [x0070](slices/x0070-headless-trust-wan-e2e.md) done** |
| **P1-4** | **world rate limit shared 化 + Room per-owner 上限** | improvement P1-3。**→ [x0071](slices/x0071-world-rate-limit-room-cap.md) done**。P3-7（BLAKE3）は **マージしない** |
| **P1-5** | **launcher discovery**（host restart 後の IPC 再通知） | improvement P1-4。**→ [x0072](slices/x0072-launcher-discovery.md) done** |

---

## P2 — inventory ladder 続き（B+2 → C+3）

Wave 1 完了後の本線。順序は [inventory-ladder.md](inventory-ladder.md) 固定。

| Wave | Slice | 到達 |
|:---|:---|:---|
| **2–4** | x0059–x0065 | **done**（Drive/Vcs/Prefab API；host UI 配線は薄皮） |

ラダー外（Wave 4 後・独立帯）: Mail / Comms / Shopfront。

---

## P3 — 品質・衛生（小さく並行可）

improvement-plan P2 帯。本線を止めないが放置すると DX/運用が腐る。

- reserved reaper / orphan CAS GC（**done: x0074**）
- BLAKE3・seal-hash の常駐 Port（**done: x0075**）
- auth Keys `:persistent_term`（**done: x0073**）、host `room_token.rs` / client `main.rs` 分割（**done: x0076**）
- bind バケット送信元分割（**done: x0077**）、`tos_agreed` UI 同意（**done**: チェックボックス必須）
- prod compose publish 既定 loopback（**done: x0073**）、cargo deny launcher/relay（**done: x0073 / x0068**）
- mTLS 鍵 temp の権限・掃除（**done: x0078**）、腐った smoke（`smoke-host-token` **done: x0073**）
- DemoScene 共有テストベクタ（**done: x0079**）、auth/world/relay metrics + `jti` 相関（**done: x007a**）

---

## P4 — 目標アーキ（未実装・評価対象外 / 減点しない）

overview にあるがコード無し〜薄皮のみ。roadmap でも「保留」と明記されたもの含む。

| 領域 | 未着手の例 |
|:---|:---|
| **監査** | verifier Spot-Check・スマートホスト移譲・リレー SLA（Behaviour 注入点のみ） |
| **空間実行** | Formula VM、Wake-on-Join、SimState Snapshot、0秒ホスト移譲、CRDT 共同編集 |
| **体感** | フルボディ IK / SpringBone / VRM、OSC 受信エッジ、予測・補間、空間オーディオ、Dual Editor、LOD ビルド、P2P キャッシュ |
| **連合・発見** | S2S 連合、`alchemy-node-info`、Room Token 時の relay URI+mTLS セット返却の完全導線 |
| **安全弁・検疫** | クライアント Mute/Block・シェーダー/爆音安全弁；Tombstone / PDQ / KTX2 検疫（v2 仕様・OSS にはヒューリスティックを置かない） |
| **経済圏** | `market-server` / BillingProvider（BSL・ディレクトリ未設置）、relay QoS/CCU、QUIC/WebTransport |
| **配布** | インストーラ・SBOM・artifact 署名、Web/Mobile/Quest 展開 |

---

## 明示的にやらない / 後回し

| 項目 | 扱い |
|:---|:---|
| **P3-7** BLAKE3 ストリーミング + Room 上限を同じ変更で | CI が壊れないよう **マージしない**（上限だけは P1-4 で切り離し可） |
| Proprietary 検疫閾値・pgvector 詳細 | OSS ツリーに実装しない（licence-and-boundaries） |
| inventory と x0053 の同一 PR | 禁止 |

---

## 推奨着手順（要約・本作業後）

1. ~~[x0081](slices/x0081-control-plane-pop.md) 制御面 session 署名 + world HTTP PoP~~ **done**
2. ~~[x0082](slices/x0082-scene-revision-cas.md) Scene revision CAS（improvement P0-3）~~ **done**
3. ~~[x0083](slices/x0083-relay-agent-csr-only.md) relay-agent CSR-only（improvement P0-4）~~ **done**
4. ~~[x0084](slices/x0084-host-token-rate-limit.md) host-token 操作別 RL~~ **done**
5. ~~[x0085](slices/x0085-glb-decode-budget.md) GLB decode budget~~ **done**
6. assets hermetic / WAN host E2E（P1-3〜）← **いま**
7. verifier Spot-Check（保留）· Formula / market / 検疫（減点対象外）

← [Roadmap 目次](README.md)
