# x00ae — Phase 3: dirty デルタ codec（`WireDirtyAccumulator`）

> **ID:** `x00ae` · **slug:** `phase3-dirty-delta-codec` · **status:** `done` · **area:** `[sim, docs]` · **depends_on:** `[x00ad]`  
> **優先度:** [mvcc_xpbd.md](../mvcc_xpbd.md) Phase 3 残り（dirty → PROTOCOL_PIN → `sim_state`）  
> **後続（本スライス外）:** PROTOCOL_PIN 一括 · `…/sim_state` · 確定レーン `…/edit`（次空き `x00af`〜）

目標: `shared-sim` に **直前 publish 以降の dirty 合成**と **Smallest-Three 回転量子化**を置き、§4.5 の reuse クリア契約（既定 A）とスキップ net tick 蓄積をヘッドレスで Green にする。host 本番 frame の dirty 専用ペイロード切替・PROTOCOL_PIN 変更は入れない。

根拠: [mvcc_xpbd.md](../mvcc_xpbd.md) §4.5 / §5 Phase 3 · x0024（欠落は snapshot・`V_ack` 禁止）

---

## 着手ゲート（満たしていること）

| 条件 | 状態 |
|:---|:---|
| Phase 3 着手 x00ad（`SimBackend` + 接触オン） | **done** |
| ページ `dirty_entity_mask` / `mark_dirty` | **done**（x00a3+） |
| Room AEAD / wire_cap / x0024 を壊さない | 本スライスは **codec のみ**（ワイヤ契約変更なし） |

破ってはならないこと: 受信者別 `V_ack`、独自 reliable、PROTOCOL_PIN 変更、本番 frame 経路の黙ってのペイロード置換、検証スキップ。

---

## 保証する / しない（本スライス）

| 保証する | 保証しない |
|:---|:---|
| `WireDirtyAccumulator`（キー=`PageId.0`・容量=アリーナ **プール**・契約 A） | host 30Hz 本番 dirty 配信切替 |
| `absorb` / `clear_slot` は範囲外 `Err`（黙って落とさない） | PROTOCOL_PIN（`sim_tick` / `holder` / `acked_intent_seq` 等） |
| `extract_dirty_records_into`（`acc & active`・`entity_of` は呼び出し側が埋める） | 隔離／削除の dirty 姿勢配信（§4.7 / snapshot） |
| Smallest-Three pack/unpack（`q ≡ -q` 同一姿勢） | `…/sim_state` / `…/edit` |
| 回帰: skip-tick · reuse · OOR · quarantine skip · pack 往復 | 冗長窓 K>1（Q6: まず K=1） |
| `#![forbid(unsafe_code)]` 維持 | Spatial Interest / 拘束ソルバー SIMD |

---

## 実装メモ

* `client/shared-sim/src/delta_codec.rs` — §4.5 API
* `WireDirtyAccumulator::for_arena(&arena)` を host 本線の正とする（`new(64)` 禁止；`from_arena_capacity` はテスト／低レベル）
* `absorb` / `clear_slot` → `Result<(), DirtyAccumError>`
* `EntityDeltaRecord` · `pack_quaternion_smallest_three` · `unpack_quaternion_smallest_three`
* キー契約 A: プール index 再利用前に `clear_slot`；fork 先 absorb は新 `PageId`
* 隔離／despawn は `acc & active` で pose レーン非掲載（§4.7 / x0024）
* テスト: `dirty_survives_skipped_net_tick` · `reuse_clears_stale_dirty` · `absorb_out_of_range_is_err` · `extract_skips_inactive_*` · `extract_emits_entity_of_verbatim` · pack 往復

---

## Phase

| Phase | 内容 | 状態 |
|:---|:---|:---|
| 0 | Slice + roadmap 目次 | **done** |
| 1 | `delta_codec` + Smallest-Three | **done** |
| 2 | 蓄積 / reuse / extract 回帰 | **done** |

---

## 完了条件

1. `cargo test -p alchemy-shared-sim --lib delta_codec` Green — **done**
2. PROTOCOL_PIN / host 本番 dirty 切替 / `sim_state` / `…/edit` を入れない — **done**
3. roadmap 目次と本ファイル `status: done` — **done**

---

## 起動・検証メモ

```powershell
cargo test -p alchemy-shared-sim --lib delta_codec
```

推奨ブランチ: `feat/x00ae-phase3-dirty-delta-codec`（未作成なら `main` 上の作業差分として進めてよい；コミット／PR 時に切る）
