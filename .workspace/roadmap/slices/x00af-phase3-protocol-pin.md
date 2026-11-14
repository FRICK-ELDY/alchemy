# x00af — Phase 3: PROTOCOL_PIN 一括（sim_tick / holder / release）

> **ID:** `x00af` · **slug:** `phase3-protocol-pin` · **status:** `done` · **area:** `[protocol, host, client, docs]` · **depends_on:** `[x00ae]`  
> **優先度:** [mvcc_xpbd.md](../mvcc_xpbd.md) Phase 3 残り（PROTOCOL_PIN → `sim_state`）  
> **後続:** `…/sim_state`（次空き `x00ag`）·（並走可）確定レーン `…/edit`

目標: Phase 3 で必要なワイヤ拡張を **PROTOCOL_PIN 1 回**で固定する。`sim_tick` / `redundancy` / `acked_intent_seq` / `Entity.holder` / Intent `release_v*/w*` を proto に載せ、host / app は既定値または最小配線で encode・decode する。

根拠: [mvcc_xpbd.md](../mvcc_xpbd.md) §4.5 / §5 Phase 3 · [mvcc_xpbd_review_proposal.md](../mvcc_xpbd_review_proposal.md) 指摘 1/3/4 · x0024（`tick` = net_tick・欠落は snapshot）

---

## 着手ゲート（満たしていること）

| 条件 | 状態 |
|:---|:---|
| dirty codec x00ae | **done** |
| host `SimBackend` + 接触オン x00ad | **done** |
| Room AEAD / wire_cap / x0024 を壊さない | 本スライスで **契約維持**（`V_ack` 禁止） |

破ってはならないこと: 受信者別 `V_ack`、独自トランスポート、検証スキップ、ピンを複数回に分割して破壊的変更を撒く、`…/sim_state` / `…/edit` / host 30Hz dirty 本番切替をこのスライスに混ぜる。

---

## 保証する / しない（本スライス）

| 保証する | 保証しない |
|:---|:---|
| `PROTOCOL_PIN` を `…-x00af` 世代へ 1 回 bump | `…/sim_state` トピック / crash last-good 本番配線 |
| `RenderFrame.sim_tick`（物理ティック；`tick` = net_tick） | 冗長窓 K>1（Q6: まず K=1；`redundancy=0` = 従来合成） |
| `RenderFrame.redundancy` フィールド存在 | host dirty codec 本番 30Hz 切替 |
| `RenderFrame.acked_intent_seq`（slot index → last applied intent_seq） | 手元予測の Snap 判定を本線で差し替え（配線のみ） |
| `Entity.holder`（0=なし、1..=32 = slot+1） | 投げ impulse の物理適用（検疫・クランプのみ） |
| `InputEvent.release_v*/w*`（クライアント計算・host 有限＋上限クランプ→overlay 保存） | 投げ impulse の物理適用（overlay 保管のみ） |
| `acked_intent_seq` = overlay 適用済み seq（**net tick の pose サンプル**と同期；mid-net は `last_wire_acked` で遅延） | `SessionBinding.last_seq` をそのまま ack すること |
| decode limits: `acked_intent_seq` ≤ `USER_ROOT_SLOTS_MAX` | Spatial Interest / 拘束 SIMD / `mvcc_xpbd` 既定切替 |
| 既存 frame / Intent / smoke 回帰 Green | 確定レーン `…/edit` |

---

## 実装メモ

* `protocol/proto/render_frame.proto` — field 6–8 / Entity.holder=16
* `protocol/proto/input_events.proto` — `release_vx..wz` = 15–20
* `PROTOCOL_PIN` = `alchemy-protocol/0.22.0-x00af`
* helpers: `entity_holder_from_slot` / `entity_holder_slot` / `clamp_release_velocity`
* host Presence `PoseOverride`: `applied_intent_seq` + clamped `release_*`（overlay 受理時）
* host Frame: `fill_acked_intent_seq_from_overlays`（tick 先頭スナップショット）+ `Pose.holder`（`grabs` 逆引き）
* mid-net `removes_only`: `last_wire_acked` / `last_wire_sim_tick` 維持；coalesce も prior poses↔sim_tick↔ack を対で残す
* app: Intent encode に release フィールド（既定 0）；受信側はフィールド読取可能
* 量子化ポーズは **本スライスでは追加しない**（dirty 本番切替スライスで可）
* DemoScene vector / world `protocol_pin` / `DEMO_SCENE_VECTOR_BLAKE3` 追随

---

## Phase

| Phase | 内容 | 状態 |
|:---|:---|:---|
| 0 | Slice + roadmap 目次 | **done** |
| 1 | proto + PROTOCOL_PIN + helpers + decode limits | **done** |
| 2 | host / app 最小配線 | **done** |
| 3 | 回帰 Green + roadmap 次手更新 | **done** |

---

## 完了条件

1. `PROTOCOL_PIN` が `x00af` 世代で、旧 pin と不一致 — **done**
2. frame / input roundtrip が新フィールドを往復 — **done**
3. host が `sim_tick` / `holder` / `acked_intent_seq` を publish に載せる — **done**
4. `release_*` の NaN / 過大値が有限＋上限にクランプされる — **done**（適用は非保証）
5. `cargo test -p alchemy-protocol` / host-server / client 関連 Green — **done**
6. `…/sim_state` / `…/edit` / dirty 本番切替を入れない — **done**
7. roadmap 目次の次 = `sim_state`（`x00ag`）— **done**

---

## 起動・検証メモ

```powershell
cargo test -p alchemy-protocol
# from client/
cargo test -p host-server --bin host-server
cargo test -p client --bin client
```

推奨ブランチ: `feat/x00af-phase3-protocol-pin`
