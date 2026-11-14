# x0094 — Hierarchy 仮想 Lens 最小（euler_order → Settings）

> **ID:** `x0094` · **slug:** `hierarchy-virtual-lens` · **status:** `done` · **area:** `[client, system_ui, sim]` · **depends_on:** `[x008e, x0093]`  
> **優先度:** Zero-Base Spatial UI 本線（Lens の Hierarchy 最小）  
> **正本仕様:** [alchemy_zero_base_spatial_ui_lens_uix_architecture.md](../alchemy_zero_base_spatial_ui_lens_uix_architecture.md) §4.3 / Step 2 の Hierarchy 部分のみ

目標: x008e の `hierarchy_visible_rows`（`euler_order` + `euler_tout` 折りたたみジャンプ）を **System UI Settings** に薄い Hierarchy パネルとして配線する。選択行は既存 Tape X-Ray に連動する。

根拠: roadmap「現在の次」· [x0093](x0093-xpbd-wire-authority.md)「やらない／次」· Lens アーキ Step 2（Uber-SDF / Inspector Zero-String は本 Slice 外）

---

## 保証する / しない

| 保証する | 保証しない |
|:---|:---|
| Frame 鏡写し後、client `EpochViewState.topology` を **平坦ルート**（全 `parent_of = MAX`）で再ベイク | host `SlotGraph` / held 親子のワイヤ同期・PROTOCOL_PIN 変更 |
| Settings に仮想スクロール Hierarchy（窓最大 8 行）を表示 | Spatial Lens 空間パネル（1 パネル＝1 行板）・Uber-SDF / MSDF |
| 折りたたみは `euler_tout` 経由 O(1) スキップ（親子が載ったとき有効） | Inspector Zero-String・NodeGraph・Tear-Off・CRDT パネル同期 |
| 行クリックで `xray_row` を設定し Tape X-Ray と連動 | Resonite レーザー Stickiness / 角度変調 |
| | Dual-AST Kernel / Atom Hydration / TPS・Live |

---

## 設計（要点）

```
Applied Frame
    │
    ▼
epoch_mirror::ingest → sync_topology_flat_roots(curr.len)  // len 不変なら no-op
    │
    ▼
ingest → clear_hierarchy_collapsed_rows(tombstone∪alloc) → scrub_* 
SystemUi::pull_hierarchy / pull_epoch_xray(refresh_text)（Settings 開時のみ）
  hierarchy_panel_rows（collapsed/topo を短時間 lock・clone なし）
  X-Ray: dirty は XrayStructureKey；pose 文字列は Relayout / 構造変化時のみ
    │
    ├─ click / scroll → apply_hierarchy_relayout（Desktop/XR 共通）
    └─ collapse → apply_hierarchy_collapse_toggle → 同上
```

- Topology ソース本 Slice: **client 局所・平坦**。真の親子（held / Prefab 多子）は後続 Slice。
- Hierarchy / X-Ray とも Layout dirty にライブ座標を載せない。X-Ray pose は構造変化または Relayout 時に画面へ載る（毎フレーム追従は非要件）。
- `hierarchy_collapsed` は `EpochViewState`（tombstone / freelist alloc で明示クリア + scrub）。
- 描画は既存 `LayoutTree`。`GpuUiPrim` は置かない。

---

## Phase

| Phase | 内容 | 状態 |
|:---|:---|:---|
| 0 | Slice + roadmap 目次 | **done** |
| 1 | `sync_topology_flat_roots` + ingest 配線 + 単体 | **done** |
| 2 | Settings Hierarchy パネル（行・scroll・collapse・X-Ray 連動） | **done** |
| 3 | `cargo test -p client` / shared-sim 緑・完了条件 | **done** |

---

## 完了条件

1. `cargo test -p alchemy-shared-sim` および `cargo test -p client` 緑
2. ingest 後 `topology.euler_order` が Curr 行長と整合（平坦ルート；len 不変は no-op）
3. `hierarchy_visible_rows` / `_filtered` 折りたたみ回帰が維持され、Settings 行クリックが `xray_row` を更新すること
4. Hierarchy の Layout dirty がライブ座標で毎フレーム発火しないこと
5. X-Ray の Layout dirty も structure key（row/mask/depth）のみで、pose / morton では発火しないこと（Settings ゲート；pose 文字列は Relayout / 構造変化時のみ確保）
6. ingest の tombstone / freelist alloc で `hierarchy_collapsed` の該当 `row_id` が落ちること（`ingest_applied_world` 経由の回帰テストあり）
7. Uber-SDF / CRDT Lens パネル / PROTOCOL_PIN 変更を入れていないこと

## やらない（本 Slice）／次

- Uber-SDF / `GpuUiPrim` / CJK アトラス（Lens Step 1 / 4）— 空間本線一時停止
- Inspector 数値 Zero-String・NodeGraph — 同上
- host→client 親子（SlotGraph / Prefab）同期 — 同上
- wire pose ダブルバッファ → **[x0095](x0095-wire-pose-double-buffer.md)** **done**
- TPS / Live カメラ

## 関連

- [x008e-zero-base-spatial-lifecycle.md](x008e-zero-base-spatial-lifecycle.md)（S-28 API）
- [alchemy_zero_base_spatial_ui_lens_uix_architecture.md](../alchemy_zero_base_spatial_ui_lens_uix_architecture.md)
- [x0093-xpbd-wire-authority.md](x0093-xpbd-wire-authority.md)

← [目次](../README.md)
