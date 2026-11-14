# x00b1 — Phase 3: host dirty 30Hz 本番切替

> **ID:** `x00b1` · **slug:** `phase3-dirty-30hz-cutover` · **status:** `done` · **area:** `[host, sim, docs]` · **depends_on:** `[x00ae, x00af, x00b0]`  
> **優先度:** [mvcc_xpbd.md](../mvcc_xpbd.md) Phase 3 残り（dirty 本番 → 並走可 `…/edit`）  
> **後続:**（並走可）確定レーン `…/edit` · Spatial Interest は Phase 3 DoD 後 · 既定 backend を `mvcc_xpbd` に切替は別スライス

目標: x00ae の `WireDirtyAccumulator` / `extract_dirty_records_into` を **host 権威ループの 30Hz publish** に配線する。60Hz 物理のうち送らない tick の dirty を OR 蓄積し、pack 時に送った bit を in-flight 化し（成功で確定破棄・空/失敗で restore）、pack 後の再 dirty を消さない。欠落は x0024 snapshot（`V_ack` 禁止）。

根拠: [mvcc_xpbd.md](../mvcc_xpbd.md) §4.5 / §5 Phase 3 · [x00ae](x00ae-phase3-dirty-delta-codec.md) · [x00af](x00af-phase3-protocol-pin.md)（量子化ポーズは本スライスで **任意**）

---

## 着手ゲート（満たしていること）

| 条件 | 状態 |
|:---|:---|
| dirty codec x00ae | **done** |
| PROTOCOL_PIN x00af（`sim_tick` / `holder` / `acked_intent_seq`） | **done** |
| `…/sim_state` x00b0 | **done** |
| Room AEAD / wire_cap / x0024 を壊さない | 本スライスで **契約維持** |

破ってはならないこと: 受信者別 `V_ack`、独自トランスポート、検証スキップ、`…/edit`、Spatial Interest、空デルタで `net_tick` を進めること、`WireDirtyAccumulator::new(64)` 固定（プール容量と不一致）。

---

## 保証する / しない（本スライス）

| 保証する | 保証しない |
|:---|:---|
| `mvcc_xpbd` で `WireDirtyAccumulator` を host が保持し、毎物理 tick `absorb` | 既定 backend を `mvcc_xpbd` に切替 |
| スキップ net tick 分の dirty が次 publish に残る（host 回帰） | `redundancy` K>1（Q6: 当面 `0` または区間 1） |
| pack 時に **送った bit だけ** `detach_bits`（in-flight）；成功で破棄、空/失敗で `restore_bits`。pack 後の再 dirty は acc に残る | 量子化ポーズの PROTOCOL_PIN bump（任意；本線は既存 f64 `Entity` で可） |
| 単一ライブ page でも契約 A: slot 0 + 将来 arena 再利用時は `clear_slot` | 本スライスでのフル `MvccPageArena` 履歴コミット本番化 |
| `epoch_xpbd` は既存 `last_sent` pose_delta を退行させない | Epoch 二重管理の完全解消（page-only 権威）— 後続で畳む |
| mid-net `removes_only` / coalesce / 空デルタ非 publish（x0024）維持 | `…/edit` / Spatial Interest / Spot-Check Q2 |
| Room AEAD・署名・finite 検疫を迂回しない | クロスアーキビット一致 |

---

## 実装メモ

* `HostMvccState`: `wire_dirty: WireDirtyAccumulator`（当面 `from_arena_capacity(1)`、slot=`0`）+ `entity_of: [u32; ENTITIES_PER_PAGE]` + extract scratch
* `step_mvcc_page` 後（page dirty が立っている間）に `absorb(0, &page)`；page 側 dirty クリアは absorb 後でよい
* net tick（`PHYSICS_PER_NET`）: `mvcc_xpbd` なら `extract_dirty_records_into` → host `Pose`（f64 TRS・scale/color/holder は既存マップ）へ変換し `FrameSample.poses` を **dirty 集合**にする。`epoch_xpbd` は現行 `fill_wire_poses_from_sim`
* in-flight 契約: `fill_wire_dirty_poses` は **emit した local の bit だけ** `detach_bits` し `dirty_inflight_mask` に載せる。成功 put は mask 破棄（再 dirty は acc 側に残る）。空デルタ / gate 閉は `wire_dirty_restores` → 次 tick 先頭で `restore_bits`
* dirty↔dirty coalesce: poses は entity_id LWW union、inflight mask は OR。dirty→full も prev inflight を引き継ぐ
* `FrameSample.dirty_authored` で worker の `last_sent` pose_changed を bypass
* user_root は Presence occupied slot のみ（full-scan と同集合；落とされた bit は detach しない）。inactive / 空き slot の sticky bit 刈り取りは本スライス非保証（reclaim・catchup・将来 mowing）
* `entity_of` は `entity_rows` / sheet 行から同期（unset→0 を wire に出さないよう extract 前に埋める）
* 量子化: `EntityDeltaRecord.packed_quat` は内部利用可；wire は当面 unpack→f64（PIN 据え置き）。PIN bump するなら本スライス末尾で 1 回に限る
* 回帰: host 版 `dirty_survives_skipped_net_tick`（正本名は [mvcc_xpbd.md](../mvcc_xpbd.md) §5 / x00ae と同契約；スキップ物理 tick 分の OR が次 net publish に残る）· 空デルタ非 publish · epoch smoke 退行なし ·（arena 導入時）`reuse_clears_stale_wire_dirty`

---

## Phase

| Phase | 内容 | 状態 |
|:---|:---|:---|
| 0 | Slice + roadmap 目次（採番訂正含む；実装は別 PR） | **done** |
| 1 | `HostMvccState` に accumulator + absorb / entity_of | **done** |
| 2 | net tick dirty→`FrameSample` + in-flight detach/restore（`dirty_authored`） | **done** |
| 3 | host 回帰 + docs / roadmap `done` | **done** |

---

## 完了条件

1. `ALCHEMY_SIM_BACKEND=mvcc_xpbd` で、物理 tick のみ dirty にしたエンティティが次の net publish の upsert に載る
2. 空デルタは publish せず accumulator をクリアしない（x0024）
3. publish 成功後、当該 sample で detach した bit は acc に戻らない（pack 後の再 dirty・unoccupied/inactive sticky は残りうる）
4. `epoch_xpbd` 既存 host テスト / pose_delta 経路が Green
5. `…/edit` / Spatial Interest / 既定 backend 切替 / 検証スキップを入れない
6. roadmap 目次の次 = 並走可 `…/edit`（次空き `x00b2`）

---

## 起動・検証メモ

```powershell
# from client/
$env:ALCHEMY_SIM_BACKEND = "mvcc_xpbd"
cargo test -p host-server --bin host-server dirty_
cargo test -p host-server --bin host-server
cargo test -p alchemy-shared-sim --lib delta_codec
```

推奨ブランチ: `feat/x00b1-phase3-dirty-30hz-cutover`
