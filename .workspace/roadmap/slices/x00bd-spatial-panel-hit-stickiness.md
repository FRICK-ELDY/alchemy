# x00bd — 板ヒット + Stickiness 最小

> **ID:** `x00bd` · **slug:** `spatial-panel-hit-stickiness` · **status:** `done` · **area:** `[client, docs]` · **depends_on:** `[x00ba, x00bb]`  
> **優先度:** Zero-Base Spatial UI Step 3（最小）

目標: レーザー／look レイでパネル OBB → 局所 UV → `binding_id` を特定し、Resonite 流 Stickiness + トリガー press-lock を載せる。

## 保証する / しない

| 保証する | 保証しない |
|:---|:---|
| `ray_panel_uv` + `pick_binding_at` / `pick_panel` | Flat Morton LBVH（個人パネル数が微小なため） |
| 角度変調スムージング + Stickiness + press-lock | 3D ベジェレーザー描画・Haptics |
| Desktop look レイでホバー更新；`F` で行選択；**パネル交差中は world Grab 開始を抑制** | XR 両手フルセット；既に掴んでいる物の強制解除 |

## 完了条件

1. hit / laser 単体テスト Green
2. Desktop `tick_spatial_lens` が sticky hover を prim 色に反映
