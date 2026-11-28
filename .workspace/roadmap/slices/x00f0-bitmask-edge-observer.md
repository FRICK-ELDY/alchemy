# x00f0 — bitmask edge Observer（F0-3）

> **ID:** `x00f0` · **slug:** `bitmask-edge-observer` · **status:** `done` · **area:** `[sim, host, docs]` · **depends_on:** `[x00ef, x00da]`  
> **優先度:** 体験本線 **F0-3**（Feel wiring）  
> **体感 1 文:** Enabled ON の立ち上がりで初速が残らず、補間残像も TELEPORT で止める。

根拠: [feel_edit_experience.md](../feel_edit_experience.md) §4.1 F0-3 · [scene_store_sim_arena.md](../scene_store_sim_arena.md) Ring2 Step 1

---

## 保証する / しない（本スライス）

| 保証する | 保証しない |
|:---|:---|
| enabled `0→1` 立ち上がり検知（`(curr ^ prev) & curr`） | attached 立ち下がり破棄の全コンポーネント配線 |
| 立ち上がり slot へ Carry-Over 後 `STATE_TELEPORT_THIS_TICK` | SoftLease との二重化（物理掴み SSoT は `holder_slot`） |
| sheet `frame_delta` ゼロ＋Prev 位置同期（初速リセット） | Enabled を Scene Store から自動供給する本線（patch 経路は既存） |
| Arena 単体テスト Green | F0-4 PendingLocalOps / F1 Inspector |

## 完了条件

1. `SimArena::take_enabled_rising_edges_into` が一度だけ立ち上がりを返す（caller バッファ再利用）。
2. host `step_dt` が rising で TELEPORT stamp ＋ delta クリア（propagate / `write_frame_deltas` の**後**）。
3. 新規クレートなし。roadmap 更新。

## 触ったファイル

- `client/shared-sim/src/sim_arena/arena.rs` — prev enabled + Observer API + テスト
- `client/host-server/src/sim.rs` — Step 1 配線・`enabled_rising_scratch`
- [feel_edit_experience.md](../feel_edit_experience.md) / [README.md](../README.md)

← [目次](../README.md)
