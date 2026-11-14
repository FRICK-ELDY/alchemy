# x00ac — HotPhysicsPage 向け `interp.wgsl` + naga 検証

> **ID:** `x00ac` · **slug:** `interp-wgsl` · **status:** `done` · **area:** `[client, docs]` · **depends_on:** `[x00aa]`  
> **優先度:** [mvcc_xpbd.md](../mvcc_xpbd.md) Phase 2 ゲート（シェーダ検証）

目標: `GpuTransferableSoA` レイアウト契約の `interp.wgsl` を置き、wgpu 再エクスポートの naga でパース検証する。Epoch インライン WGSL 描画は当面維持。

## 保証する / しない

| 保証する | 保証しない |
|:---|:---|
| `drot_*` / `inv_inertia_*` を含む SoA フィールド順 | Epoch draw の即時置換 |
| naga `parse_str` + entry `cs_interpolate_entities` | host ページ upload 本線 |
| `cell_rel_offset` uniform | マルチページ Directory upload |

## 完了条件

1. `cargo test -p client render_wgpu` Green（naga 検証含む）。
2. Phase 2 DoD（jitter + 予測 + WGSL）を閉じ、roadmap 目次 `done`。
