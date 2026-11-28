# x00ef — Depth Bucket 階層伝播（F0-2）

> **ID:** `x00ef` · **slug:** `depth-bucket-hierarchy` · **status:** `done` · **area:** `[sim, host, docs]` · **depends_on:** `[x00eb, x00dc]`  
> **優先度:** 体験本線 **F0-2**（Feel wiring）  
> **体感 1 文:** 親を動かすと子のワールド座標が追従し、Reparent しても見た目位置が跳ばない。

根拠: [feel_edit_experience.md](../feel_edit_experience.md) §4.1 F0-2 · [scene_store_sim_arena.md](../scene_store_sim_arena.md) Ring2 Step 3 / §4.3

---

## 保証する / しない（本スライス）

| 保証する | 保証しない |
|:---|:---|
| 空間 Slot について tick 内 1 回、depth bucket 浅→深で親→子 **pos/scale** 確定（host Step 3） | 完全 TRS / クォータニオン親子合成（translation-first；回転は defer） |
| SetParent 成功時 **ワールド維持 → 局所再計算**（契約 A）；非有限時 TELEPORT stamp | dirty 部分木だけの O(dirty) 伝播最適化 |
| 親ワールド変更＋子局所不変 → 子ワールド追従 | client Hierarchy の host 親ミラー（x0094 / flat_roots） |
| dirty な親付き行は world→local 再計算（Grab / XPBD / Intent 可。kinematic-only ではない） | 伝播で `inv_mass` / `rotation` / `frame_delta` / `flags` を書き換えること |
| **depth 順一巡**: 祖先確定 → dirty なら finalized 親基準で invert → 当該行 pos/scale 確定 | 伝播前の親 world で invert してから別パスで compose（祖先デルタずれ） |
| x00eb TELEPORT / GPU snap との共存 | LiveEdit → `SimPatch` 一経路化（二重適用防止のため非対象） |
| `row_id` 不変・`FlatTopology` は間接配列のみ | AvatarRig Slot 化 |

## 完了条件

1. host `step_dt` が `propagate_world_transforms` を呼び、親移動で子ワールドが追従する単体テスト Green。
2. SetParent（A）: ワールド座標不変・局所だけ変わる。
3. 既存 x00eb / teleport 回帰 Green。新規クレートなし。
4. roadmap 目次・feel 「次の着手」更新。

## 触ったファイル

- `client/shared-sim/src/topology.rs` — `recompute_local_from_world`；伝播 API 公開
- `client/shared-sim/src/lib.rs` — re-export
- `client/host-server/src/sim.rs` — `local_transforms`、Reparent A、Step 3
- `client/shared-sim/src/sim_arena/mod.rs` — Step 3 契約コメント
- [feel_edit_experience.md](../feel_edit_experience.md) / [README.md](../README.md)

← [目次](../README.md)
