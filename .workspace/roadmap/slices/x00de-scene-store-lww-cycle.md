# x00de — scene_store LWW ＋ 循環防止

> **ID:** `x00de` · **slug:** `scene-store-lww-cycle` · **status:** `planned` · **area:** `[sim, client]` · **depends_on:** `[x00dd]`  
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
| `OpTimestamp`；lamport ドリフトは契約コメント／最小クランプ可 | TRASH_UUID 削除モデル、SoftLease、`op_chain_hash` |

## 触るファイル（予定）

- `client/shared-sim/src/scene_store/*`

## 完了条件

1. 循環 Reject・LWW の単体テスト Green。
2. LiveEditLedger facade・edit ワイヤ変更なし。
