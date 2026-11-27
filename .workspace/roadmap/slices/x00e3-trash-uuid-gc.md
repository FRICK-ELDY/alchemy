# x00e3 — TRASH_UUID MoveNode ＋ Trash GC

> **ID:** `x00e3` · **slug:** `trash-uuid-gc` · **status:** `planned` · **area:** `[sim, host]` · **depends_on:** `[x00e1]`  
> **優先度:** 品質本線 Phase 1c  
> **後続:** [x00e4](x00e4-three-layer-ids.md)

目標: 削除モデルを現行 `pending_despawns` から **`TRASH_UUID` への `MoveNode` + `TrashStore`（件数/TTL GC）** へ移行する。GC 済み Undo は No-op。

根拠: [scene_store_sim_arena.md](../scene_store_sim_arena.md) §3.2 / §6 `TRASH_UUID` / §7 Phase 1c

---

## 保証する / しない（本スライス）

| 保証する | 保証しない |
|:---|:---|
| 論理削除 = TRASH への MoveNode；Trash GC | SoftLease 必須、StableUuid 全経路化 |
| GC 後 Undo No-op | Spatial VCS 永続スキーマ全面書き換え |

## 完了条件

1. TRASH MoveNode + GC + Undo No-op の単体／統合テスト。
2. 1b の pending_despawns からの移行パスを文書化。
