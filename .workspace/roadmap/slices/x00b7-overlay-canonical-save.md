# x00b7 — overlay→canonical 明示 Save（B）

> **ID:** `x00b7` · **slug:** `overlay-canonical-save` · **status:** `done` · **area:** `[world, host, client, protocol, system_ui, docs]` · **depends_on:** `[x009e, x009d, x00b3, x00b6]`  
> **優先度:** [mvcc_xpbd.md](../archive/2026-11-27/mvcc_xpbd.md) §4.7 明示 Save（箱の永続）  
> **後続:** 親子 / SetField 永続は [x00b8](x00b8-save-parent-setfield-persist.md)；Spawn 即 CAS 見直し（C）· Save As / Copy · Spatial VCS

目標: Room の **Save Changes** で `session_overlay` を canonical `scene` へ原子マージし overlay を空にする。dormant / overlay clear 後も箱が残る。親子・SetField の永続は本スライスではやらない。

## 保証する / しない

| 保証する | 保証しない |
|:---|:---|
| Save 成功後、overlay clear しても元 overlay mesh は canonical に残る | parent / SetField の永続（A） |
| 非 writer active host の Save は 403 `scene_writer_forbidden` | Save As / Copy / Autosave |
| packed CAS で scene と overlay を同時更新 | Spawn 即 overlay CAS 廃止（C） |
| 空 overlay Save は冪等（rev 不変） | Spatial VCS / Drive commit |
| `EditProposal` Save → host → world；tip は進めない | |

## 設計確定

| 項目 | 決定 |
|:---|:---|
| API | `POST /api/v1/rooms/:id/scene/save` `{ expected_revision }` |
| 認可 | host Token + PoP + active_host + scene writer |
| マージ | `canon.entries ++ overlay.entries`（`entity_id` 維持）、canon rev+1、overlay clear + overlay rev+1 |
| ワイヤ | `EditSave` op · `PROTOCOL_PIN` = `alchemy-protocol/0.24.0-x00b7` |
| tip | Save 成功でも `live_scene_rev` は据え置き |

## Phase

| Phase | 内容 | 状態 |
|:---|:---|:---|
| 0 | Slice + 目次 | done |
| 1 | world `save_overlay_to_canonical` + HTTP + tests | done |
| 2 | protocol EditSave + PIN | done |
| 3 | host room_scene + edit 配線 | done |
| 4 | Room Save Changes UI | done |

## 起動確認

1. inventory spawn → Room Save → overlay clear 相当でも GET scene に箱が残る  
2. 非 writer Save → 403  
3. 空 overlay Save → 成功・rev 不変  
