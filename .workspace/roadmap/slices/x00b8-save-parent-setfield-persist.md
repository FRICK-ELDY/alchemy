# x00b8 — Save 時 parent / SetField 永続（A）

> **ID:** `x00b8` · **slug:** `save-parent-setfield-persist` · **status:** `done` · **area:** `[protocol, world, host, docs]` · **depends_on:** `[x00b7, x00b3]`  
> **優先度:** [mvcc_xpbd.md](../archive/2026-11-27/mvcc_xpbd.md) §4.7 案 B の非対称解消（箱は残る・親子は消える）  
> **後続:** [x00b9](x00b9-spawn-without-live-cas.md) Spawn 即 CAS 見直し（C）· Save As / Copy · Spatial VCS · Undo UI

目標: **Save 時のみ**、host live ledger の parent / flags / color / blake3 を canonical scene に折り込む。Reparent/SetField のライブ都度 world CAS はしない（x00b3 案 B 維持）。

## 保証する / しない

| 保証する | 保証しない |
|:---|:---|
| Save 後、Leave/dormant しても parent / flags / color / blake3 が canonical に残る | ライブ都度 CAS |
| 未知 `entity_id` の patch → 422 `invalid_entry`（既存 scene entry と同じ） | pose（x,y,z,q,s）の Save 永続 |
| 循環 parent → 422 `parent_cycle` | Save As / Spatial VCS / Undo UI |
| ManifestSeal 入力に `parent_entity_id` を含む（親子差分は seal で検知） | |
| empty overlay + 空 patches は冪等（rev 不変） | Spawn 即 CAS 廃止（C） |
| tip は Save でも据え置き（x00b7 と同じ） | |

## 設計確定

| 項目 | 決定 |
|:---|:---|
| ワイヤ | `SceneEntry.parent_entity_id`（0 = root）· `PROTOCOL_PIN` = `…-x00b8` |
| API | `POST …/scene/save` body に任意 `parents` / `fields`（x00b7 拡張） |
| 原子性 | overlay merge → patches 適用 → 1 packed CAS |
| host | EditSave 時に `LiveEditLedger` を snapshot して同梱；boot で scene から seed |

## Phase

| Phase | 内容 | 状態 |
|:---|:---|:---|
| 0 | Slice + 目次 | done |
| 1 | proto + PIN + demo seal | done |
| 2 | world patches + tests | done |
| 3 | host fold + boot seed | done |

## 起動確認

1. spawn → Reparent → SetField(color) → Save → Leave → 再 Host：親子と色が残る  
2. 循環 parent patch → 422 `parent_cycle` 
3. 空 overlay + 変更なし Save → rev 不変  
