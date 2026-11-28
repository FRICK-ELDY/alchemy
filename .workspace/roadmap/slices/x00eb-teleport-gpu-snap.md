# x00eb — TELEPORT → GPU snap（F0-1）

> **ID:** `x00eb` · **slug:** `teleport-gpu-snap` · **status:** `done` · **area:** `[sim, host, client]` · **depends_on:** `[x00dc, x00e6]`  
> **優先度:** 体験本線 **F0-1**（Feel wiring）  
> **体感 1 文:** Reparent・ワープ直後に補間残像が消え、Curr（Ring3 Published 面）へ即スナップする。

根拠: [feel_edit_experience.md](../feel_edit_experience.md) §4.1 F0-1 · [scene_store_sim_arena.md](../scene_store_sim_arena.md) §4.2 / Ring3

---

## 保証する / しない（本スライス）

| 保証する | 保証しない |
|:---|:---|
| `WorldTransformf64::FLAG_TELEPORT` と Arena `STATE_TELEPORT_THIS_TICK` の写像 | RenderFrame ワイヤへの teleport ビット追加 |
| 出版直前に Arena Work → Curr sheet へ teleport を **sparse stamp**（`this_tick_dirty`；live sheet は汚さない） | Depth Bucket 階層伝播（→ F0-2）/ bitmask edge Observer（→ F0-3） |
| Epoch `lerp_gpu_rows` ＋ `INTERPOLATE_WGSL` が Curr teleport 時に Lerp/Slerp をスキップし Curr へスナップ | Guest Frame 経路だけのヒューリスティック残像根絶（ワイヤ非保証） |
| host の live reparent 成功後に該当行へ `write_teleport`（**sheet→Arena sync / resize の後**） | SoftLease 見える化 / Inspector Commit（→ F1） |
| 残像回帰テスト（α=0.5 でも Curr 座標；spawn+reparent 同一 tick grow） | `spawn_mesh_entry` への `pending_teleport` 積載（新規行の残像は `pack_aligned_gpu_faces` ABA 退避で担保） |
| | `PendingLocalOps::reconcile(teleport)` を sheet FLAG から自動供給（→ F0-4） |

## 完了条件

1. Curr sheet に `FLAG_TELEPORT` がある行は、CPU twin と Epoch WGSL の双方で α によらず Curr へスナップ。
2. Arena Work の teleport が `stamp_teleport_flags_from_arena_work` 経由で出版 Curr に載る（live `world.curr` へ sticky しない）。
3. host reparent 成功 → Carry-Over → **sync（resize）** → `write_teleport` → stamp → publish。
4. 関連単体テスト Green。新規クレートなし。

## 生成 vs Reparent（体感文との差分）

| 経路 | 本スライスの扱い |
|:---|:---|
| **新規生成**（`spawn_mesh_entry` 等） | `pending_teleport_rows` は積まない。Prev 欠落 / 世代不一致は既存 ABA で Prev←Curr コピーし lerp が no-op |
| **Reparent / ワープ** | FLAG + GPU snap（同一 tick で sheet が Arena より伸びても sync 後に `write_teleport`） |

## 触ったファイル

- `client/shared-sim/src/tape.rs` — `WorldTransformf64::FLAG_TELEPORT` / `is_teleport` / `set_teleport`
- `client/shared-sim/src/bridge.rs` — `stamp_teleport_flags_from_arena_work`；`epoch_sheet_from_sim_face` が teleport を写す；回帰テスト
- `client/shared-sim/src/lib.rs` — re-export
- `client/shared-sim/src/sim_arena/mod.rs` — Ring3 観測契約（x00eb）コメント
- `client/app/src/epoch_view.rs` — `GpuEpochRow.teleport`；`lerp_gpu_rows` / `INTERPOLATE_WGSL` スナップ；残像テスト
- `client/app/src/shader.wgsl` — 頂点 `EpochRow` フィールド名を `teleport` に揃える
- `client/app/src/pending_local_ops.rs` — x00eb との整合コメント
- `client/host-server/src/sim.rs` — reparent/`teleport_user_root` → dedupe queue；**sync→write_teleport→stamp**；grow 回帰テスト
- [feel_edit_experience.md](../feel_edit_experience.md) / [README.md](../README.md) / [scene_store_sim_arena.md](../scene_store_sim_arena.md) — 次着手更新

← [目次](../README.md)
