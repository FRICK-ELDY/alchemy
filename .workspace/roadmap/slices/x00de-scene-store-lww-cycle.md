# x00de — scene_store LWW ＋ 循環防止

> **ID:** `x00de` · **slug:** `scene-store-lww-cycle` · **status:** `done` · **area:** `[sim, client]` · **depends_on:** `[x00dd]`  
> **優先度:** 品質本線 Phase 1b  
> **後続:** SimPatch は [x00df](x00df-scene-store-sim-patch.md)

目標: Host-Sequenced 適用エンジンに **Property-level LWW** と **循環 `MoveNode` No-op（Reject）**、`OpTimestamp { lamport, actor_id }` を載せる。キーは引き続き `entity_id: u32`。

根拠: [scene_store_sim_arena.md](../scene_store_sim_arena.md) §3.2 / §5.3 / §7 Phase 1b

---

## 保証する / しない（本スライス）

| 保証する | 保証しない |
|:---|:---|
| 循環親設定の Reject / No-op | host 入口の JWT `sub` 強制（現行ゲート維持・本スライスで再実装しない） |
| Property-level LWW（同一 prop の後勝ち） | `SimPatch` → Arena Drain（→ [x00df](x00df-scene-store-sim-patch.md)） |
| `OpTimestamp`；lamport ドリフトは超過 Reject（梯子登りクランプ禁止） | TRASH_UUID 削除モデル、SoftLease、`op_chain_hash` |
| live 再 `SpawnNode` = parent LWW（`AppliedAsReparent`；props 保持。再生成ではない） | 「既に live なら専用 hard-error」への将来分岐は任意 |

## 触ったファイル

- `client/shared-sim/src/scene_store/timestamp.rs` — `OpTimestamp` / `MAX_LAMPORT_DRIFT`
- `client/shared-sim/src/scene_store/op.rs` — Phase 1b `CrdtOp` / `PropValue`（`entity_id`）
- `client/shared-sim/src/scene_store/store.rs` — `SceneStore` 適用エンジン（LWW / 循環 / drift）
- `client/shared-sim/src/scene_store/mod.rs` / `sparse_set.rs` / `lib.rs` — 配線・re-export
- host `LiveEditLedger` / edit ワイヤ — **変更なし**

## Phase

| Phase | 内容 | 状態 |
|:---|:---|:---|
| 0 | Slice 文書 | **done** |
| 1 | `OpTimestamp` + Property-level LWW + 循環 Reject | **done** |
| 2 | 単体テスト Green | **done** |

## 完了条件

1. 循環 Reject・LWW の単体テスト Green。
2. LiveEditLedger facade・edit ワイヤ変更なし。
