# Alchemy 未完了リスト（優先度付き / 2026-11-04 時点）

正本の進捗目次は [README.md](README.md)。本ファイルは **未完了帯の優先度ビュー**。

**既に閉じた前提（このリストから除外）:** Desktop 手通し〜x006a、inventory Wave 0–4（x0054–x0065）、CAS access 共存 peel、`tos_agreed` UI、**x0053 hotswitch**、**x006b PlaceRead**、**x006c 案 B**、**x006d host take-out**、**x006e put-back UI**、**x006f access 列廃止**、**x0070 trust WAN E2E**、**x0071 world RL / Room cap**、**x0072 launcher discovery**。**x0073** 軽量衛生 **done**。**x0074** reserved reaper / orphan CAS GC **done**。**x0075** BLAKE3 / seal-hash 常駐 Port **done**。空き ID = `x0076`。

---

## P0 — いまの次（本線）

| 優先 | ID | 内容 | 状態 |
|:---|:---|:---|:---|
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
| **P0-次** | — | P3 残衛生 or scene despawn（採番は着手時 `x0076`） | planned |
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
- bind バケット送信元分割、`tos_agreed` UI 同意（**done**: チェックボックス必須）
- prod compose publish 既定 loopback（**done: x0073**）、cargo deny launcher/relay（**done: x0073 / x0068**）
- mTLS 鍵 temp の権限・掃除、腐った smoke（`smoke-host-token` **done: x0073**）
- DemoScene 共有テストベクタ、auth/world/relay metrics + `jti` 相関

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

1. P3 残衛生（bind 送信元 / mTLS temp / DemoScene ベクタ / metrics+jti）または scene despawn
2. verifier / Formula / market / 検疫（条件: seal / gen-rev 済み）

← [Roadmap 目次](README.md)
