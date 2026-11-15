# x00bb — Spatial Hierarchy Lens

> **ID:** `x00bb` · **slug:** `spatial-hierarchy-lens` · **status:** `done` · **area:** `[client, system_ui, docs]` · **depends_on:** `[x00ba, x0094]`  
> **優先度:** Zero-Base Spatial UI Step 2（Hierarchy）  
> **後続:** [x00bc](x00bc-spatial-inspector-lens.md)

目標: x0094 の `hierarchy_panel_rows`（euler 仮想窓）を Settings `LayoutTree` ではなく **空間板の GpuUiPrim** へ投影する。個人パネルのみ（WorldRoot 非同期）。

## 保証する / しない

| 保証する | 保証しない |
|:---|:---|
| 窓分だけの Hierarchy 行 → RoundedBox + entity_id DirectNumber | MSDF ノード名ラベル |
| `I` で個人 Lens 表示切替（Desktop） | 他者へのパネル姿勢同期 |
| 折りたたみは既存 `hierarchy_collapsed`（Settings と共有・**Lens 側トグルは非保証 / Settings のみ**） | Tear-Off；Lens からの collapse 操作 |

## 操作

- `I` — Spatial Lens 表示トグル（Dash 閉時）。**開いた瞬間だけ**視線前方へパネルをスナップ（その後はワールド固定・頭追従なし）
- マウスホイール — Hierarchy スクロール（空窓まで超えないよう clamp）
- `F` — ホバー行を選択（x00bd ヒットと連動）
- 折りたたみ — Settings Hierarchy のみ（Lens は読み取り）

## 完了条件

1. `pack_personal_lens` が Hierarchy 行を prim 化する単体テスト Green
2. scene に UI 子エンティティを増やさない
