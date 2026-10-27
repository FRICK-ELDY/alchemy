# x002b — Grab OBB（描画 TRS とヒット整合）

> **ID:** `x002b` · **slug:** `grab-obb` · **status:** `done` · **area:** `[client, docs]` · **depends_on:** `[x002a]`

目標: grabbable 箱のポインタ / Grab ヒットを **AABB → OBB** にし、Frame の `rotation` + `scale`（x0029）と Desktop/XR 描画の TRS を揃える。

正本: client `world_hit.rs` · [x0029-renderframe-trs.md](x0029-renderframe-trs.md)

## 設計方針

| 項目 | 内容 |
|:---|:---|
| アルゴリズム | レイを box 局所へ（`R⁻¹ (o−c)`）。局所 AABB スラブ。半サイズ = Manifest `half_extent` × Frame scale（軸別） |
| 呼び出し | `ray_hit_grabbable_boxes*` / Desktop `pick_grabbable_desktop` / XR 既存経路（API 形は据え置き） |
| PIN | **据え置き**（ワイヤ不変） |

## 保証する

- 回転した箱でも見た目の角とヒットが一致する（同一 pose TRS）
- identity 回転時は従来 AABB と同等の `t`
- Desktop pick pad: **`(Manifest half + PAD) × Frame scale`**（pad は scale 前。scale 後加算ではない）
- 負スケールは `|scale|` で鏡映と同一のヒット体積

## 保証しない

- mesh / 三角形ヒット
- host Grab の aim **回転**追従（translation のみのまま）
- 非一様 Manifest half（現状はスカラー `half_extent`）

## 成功条件

1. `cd client && cargo test -p client --tests world_hit` で OBB 回帰緑（45° yaw・非一様 scale 含む）  
2. roadmap / x0029 非保証の「ヒット回転未反映」を解消表記（実装 PR と同時に `status: done`）  

← [目次](../README.md)
