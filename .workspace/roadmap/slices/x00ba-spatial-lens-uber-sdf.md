# x00ba — Spatial Lens Uber-SDF / GpuUiPrim 最小

> **ID:** `x00ba` · **slug:** `spatial-lens-uber-sdf` · **status:** `done` · **area:** `[client, docs]` · **depends_on:** `[x0094, x00ac]`  
> **優先度:** [alchemy_zero_base_spatial_ui_lens_uix_architecture.md](../alchemy_zero_base_spatial_ui_lens_uix_architecture.md) Step 1  
> **後続:** [x00bb](x00bb-spatial-hierarchy-lens.md)

目標: 空間 UI を「部品＝Slot」にせず、`GpuUiPrim`（64B）+ `GpuPanelHeader` + Uber-SDF WGSL の 1 DrawCall パイプラインを置く。Dash `LayoutTree` は触らない。

## 保証する / しない

| 保証する | 保証しない |
|:---|:---|
| `GpuUiPrim` / `GpuPanelHeader` Pod レイアウト + DirectNumber 固定小数パック | MSDF アトラス本実装・CJK |
| `uber_sdf.wgsl` の naga 検証（vs/fs） | XR CompositionLayerQuad |
| Desktop で個人パネル描画パス（`SpatialLensGpu`） | scene / overlay に UI 子エンティティ |

## 完了条件

1. `cargo test -p client spatial_lens` Green（naga + size 契約）
2. Dash / Save / Spawn 経路を変更しない
3. roadmap 目次 `done`
