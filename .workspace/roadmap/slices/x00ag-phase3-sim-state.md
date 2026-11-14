# x00ag — Phase 3: `…/sim_state`（計画的 handover + crash fail-closed）

> **ID:** `x00ag` · **slug:** `phase3-sim-state` · **status:** `done` · **area:** `[protocol, host, relay, docs]` · **depends_on:** `[x00af]`  
> **優先度:** [mvcc_xpbd.md](../mvcc_xpbd.md) Phase 3 残り（`sim_state` → 並走可 `…/edit`）  
> **後続:**（並走可）確定レーン `…/edit` · host dirty 30Hz 本番切替 · Spatial Interest は Phase 3 後

目標: 計画的 handover 用に **量子化なし** `…/sim_state`（request/response・Room AEAD・host 署名）を追加し、旧 host からの丸渡しで軌道を継続する。crash は §4.8（last-good / snapshot / Manifest 冷起動禁止）。

根拠: [mvcc_xpbd.md](../mvcc_xpbd.md) §4.8 / §5 Phase 3 / Q1 採用済み · [mvcc_xpbd_review_proposal.md](../mvcc_xpbd_review_proposal.md) 指摘 7

---

## 着手ゲート（満たしていること）

| 条件 | 状態 |
|:---|:---|
| PROTOCOL_PIN x00af（`sim_tick` / `holder` / `acked_intent_seq`） | **done** |
| host `SimBackend` + 接触オン x00ad | **done** |
| dirty codec x00ae（shared-sim） | **done**（本番 30Hz 切替は本スライス外） |
| handover Q1（計画＝`sim_state`、crash＝§4.8） | **採用済み** |
| Room AEAD / wire_cap / x0024 を壊さない | 本スライスで **契約維持** |

破ってはならないこと: 独自トランスポート、検証スキップ、受信者別 `V_ack`、`…/edit`、Spatial Interest、host dirty 本番切替、unknown+snapshot 不可時の Manifest 冷起動。

---

## 保証する / しない（本スライス）

| 保証する | 保証しない |
|:---|:---|
| `sim_state_key` + relay host ACL（guest put 不可） | guest が `…/sim_state` を query すること |
| `AeadKind::SimState`（Frame/Snapshot と相互に開けない） | PROTOCOL_PIN bump（新メッセージ；pin は SimState 内で照合） |
| proto `SimState` / `SimEntity`（f64 位置・量子化なし quat・`delta`・`drot` 予約・`holder`） | `drot` の物理復元（送信 0・将来用） |
| host 署名（`host_sig`；`verify_sim_state(expected_host_pub, …)`） | 本番での旧 `host_pub` 自動調達（引数で渡す；配線は非保証） |
| `PhysicsWorld::export_sim_state` / `from_sim_state_with_backend`（sheet SSoT） | クロスアーキビット一致 |
| 計画的: 同一バイナリで軌道継続（`planned_handover_continues_trajectory`；`flags`=readiness 復元含む） | supervisor / world lease 連動の自動起動モード |
| boot fetch 完了後にのみ自 `…/sim_state` queryable を宣言 | 自 queryable を fetch 前に宣言すること |
| crash 判定: last-good 優先；1 frame → 速度 0；unknown+snapshot 不可 → FailClosed（物理・frame 非起動・冷起動しない） | app→host への last-good プロセス間受け渡し |
| 非有限・速度上限・holder 範囲の検疫拒否 | `…/edit` / dirty 30Hz 本番 / Spatial Interest |

---

## 実装メモ

* `protocol/proto/sim_state.proto` — package `alchemy.render`（既存 include に載せる）
* `sim_state_key` / `SIM_STATE_KEY_LOCAL`；`AeadKind::SimState`；`WireClass::SimState`（1MiB）
* host: `spawn_sim_state_queryable` は **boot fetch 後**；`ALCHEMY_HOST_BOOT_MODE` 未知値は Err（黙って cold にしない）
* planned: `session.get(…/sim_state)` → open → check → `room_id` → verify → `from_sim_state_with_backend`
* crash: last-good IPC 非保証；本番は旧 host `…/snapshot` を速度 0 シードとして試行（`ALCHEMY_PREV_HOST_PUB` 要）
* `flags` = readiness 低32bit；planned は検疫＋apply で復元。CrashPoseOnly は flags を 0 に正規化し apply でも readiness を触らない
* snapshot / sim_state queryable は **boot fetch 後**に宣言（fetch 中は自 queryable 無し＝意図的。late-join はその窓で一時取得不能）
* sim_state cache は publish 中の **2 net tick に 1 回（~15Hz）** 更新

---

## Phase

| Phase | 内容 | 状態 |
|:---|:---|:---|
| 0 | Slice + roadmap 目次 | **done** |
| 1 | proto + key + AEAD + decode + sign | **done** |
| 2 | export/import + queryable + boot mode | **done** |
| 3 | 回帰 Green + docs / peer-delivery | **done** |

---

## 完了条件

1. `…/sim_state` キー + host ACL + AEAD kind が Frame/Snapshot と分離
2. `planned_handover_continues_trajectory`（同一バイナリ・ビット一致）Green
3. `crash_fail_closed_unknown_without_snapshot` / `crash_handover_single_frame_velocity_zero` Green
4. `sim_state_rejects_*`（署名・非有限・wrong kind）Green
5. `…/edit` / dirty 本番切替 / Spatial Interest / 検証スキップを入れない
6. roadmap 目次更新（次 = 並走可 `…/edit` または dirty 本番）

---

## 起動・検証メモ

```powershell
cargo test -p alchemy-protocol
# from client/
cargo test -p host-server --bin host-server sim_state
cargo test -p host-server --bin host-server planned_handover
```

推奨ブランチ: `feat/x00ag-phase3-sim-state`
