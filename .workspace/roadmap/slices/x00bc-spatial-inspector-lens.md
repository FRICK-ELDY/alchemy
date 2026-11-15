# x00bc — Spatial Inspector Lens（Zero-String）

> **ID:** `x00bc` · **slug:** `spatial-inspector-lens` · **status:** `done` · **area:** `[client, docs]` · **depends_on:** `[x00bb]`  
> **優先度:** Zero-Base Spatial UI Step 2（Inspector）  
> **後続:** [x00bd](x00bd-spatial-panel-hit-stickiness.md)

目標: 選択 `row_id` の pose / readiness を CPU `String` なしで `DirectNumber` として空間パネルに表示する。

## 保証する / しない

| 保証する | 保証しない |
|:---|:---|
| pos X/Y/Z + readiness bits を固定小数 DirectNumber | 数値スクラブ → `…/edit` SetField Commit |
| 選択行は既存 X-Ray（`set_xray_row`）と連動 | 全コンポーネント欄・AssetAtom 編集 |

## 完了条件

1. `pack_lens_emits_hierarchy_and_inspector_numbers` Green
2. `format!` 経路を Inspector 数値に使わないこと
