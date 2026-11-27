# x00e3 — TRASH_UUID MoveNode ＋ Trash GC

> **ID:** `x00e3` · **slug:** `trash-uuid-gc` · **status:** `done` · **area:** `[sim, host]` · **depends_on:** `[x00e1]`  
> **優先度:** 品質本線 Phase 1c  
> **後続:** [x00e4](x00e4-three-layer-ids.md)（**done**）

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

## 実装メモ（done）

- Phase 1b キー: `TRASH_ENTITY_ID = u32::MAX - 1`（`TRASH_UUID` 低 32bit）。仮想親は SparseSet 非載。
- `SceneStore::move_to_trash_sparse`（子退避 → MoveNode）；`TrashStore` FIFO（`TRASH_MAX_ENTRIES=256` / `TRASH_TTL_TICKS=3600`）。
- host `note_live_despawn`: canonical → trash + **Save 投影** `pending_despawns`；session spawn cancel → freelist hard remove。
- GC / Save clear → `remove_node_sparse`；Undo 復元失敗（missing）は `Ok(None)` No-op。

## 1b → 1c 移行パス（pending_despawns）

| 段階 | ライブ SSoT | Save `despawns` |
|:---|:---|:---|
| **1b（旧）** | `remove_node_sparse` + `pending_despawns` | `pending_despawns` そのまま |
| **1c（本スライス）** | `MoveNode` → `TRASH_ENTITY_ID` + `TrashStore` | **投影として `pending_despawns` を維持**（canonical trash 時 insert；Save 成功で clear＋hard remove） |
| **後続（任意）** | 同上 | Save が trash / tombstone を直接列挙し `pending_despawns` を廃する（本スライス外） |

ライブ編集の `contains_entity` は **非 trashed** のみ。edit ワイヤの `new_parent == TRASH_ENTITY_ID` は `NOT_FOUND`（despawn API 専用）。

## 契約メモ（レビュー確定）

- **子退避**: 親 trash 時に直下の子は live な restore parent（なければ root）へ。despawn Undo は **親ノードだけ**戻し、退避済みの子は祖父／root 配下のまま（再アタッチしない）。
- **GC と Undo**: GC hard-remove 後の Undo/Redo は常に `Ok(None)` No-op。ID 再利用は `pending_despawns` が `clear_save_pending` するまで `alloc_entity_id` が穴を避ける前提。
- **本番 despawn Undo**: HostLoop は `note_despawn_undo` を呼ばない（Intent 未配線の非保証）。
- **idempotent trash**: 既 trashed への再 `note_live_despawn` は `Trashed` のまま `pending_despawns` を維持。
- **re-seed**: trashed 行は `seed_node` 拒否。host は forget + hard remove してから seed（`clear_trashed_for_reseed`）。GC 済みで `pending_despawns` だけ残っている ID も投影を落とす。
- **HostLoop**: `note_live_despawn` が `Failed` のとき tip Accept せず `reject_edit_apply`。scene/physics 行の巻き戻しは**非保証**（sparse live のままずれうる）。
- **idempotent `note_despawn` 戻り値**: 再入場の `Some(None)` は restore parent ではない。Undo `from_parent` は初回 trash 時のみ使う。
