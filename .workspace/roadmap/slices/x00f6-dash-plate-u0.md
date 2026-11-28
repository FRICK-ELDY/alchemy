# x00f6 — U0 公式 Dash 板経路（シェル＋Chrome）

> **ID:** `x00f6` · **slug:** `dash-plate-u0` · **status:** `done` · **area:** `[client, docs]` · **surface:** `[desktop, vr]` · **depends_on:** `[x000f, x00ba, x00ec, x00ed]`  
> **優先度:** 体験本線 **U0**（UIX）  
> **体感 1 文:** XR で公式 Dash を開くと、`dash_quad` 上の Uber-SDF 板にダークスモーク＋縁 glow のシェル／Chrome が乗り、CPU 全面テクスチャだけの仮見た目から抜ける（Desktop は従来の fullscreen overlay のまま）。

根拠: [feel_edit_experience.md](../feel_edit_experience.md) §4.4 G0/U0 · §4.6 U0 · Lens 正本「1 パネル＝1 板」· [x00ec](x00ec-glass-material-g0.md) · [x00ba](x00ba-spatial-lens-uber-sdf.md)

---

## 保証する / しない（本スライス）

| 保証する | 保証しない |
|:---|:---|
| **XR:** 開いた公式 Dash ＝ **1** `GpuPanelHeader`（`dash_quad` と同ポーズ）。Shell／Chrome／タブを `RoundedBox`＋G0 glass＋T1 `MsdfGlyph` | Facet 並べ替え・CustomUIX（**Fc deferred**）；Desktop を視線前板に寄せること（→ **U0.1**） |
| **XR:** CPU `plate_chrome` で Shell／バー／タブ塗り＋タブラベルを透明／抑制し、ガラス見た目は板経路が正 | 本文 Facet の全 prim 化（→ U0.1）；装飾 Close `"X"` prim（CPU Close が正） |
| **Desktop:** 従来どおり fullscreen CPU overlay（背景ディム＋Chrome）。板経路は載せない（板≠overlay の二系統を作らない） | Desktop の Uber-SDF Dash 体感（U0.1 で本文ごと板へ寄せる） |
| ヒット: 既存 `WidgetId`／Layout 座標（XR は従来どおり `ray_hit_dash_quad` → UV） | Stickiness 新配線；LayoutTree 廃止 |
| active Shell glow＝板上 hover／press（G0）；`QUAD_*`＝`DASH_*` 単一 SSoT | G1 ブラー；新規クレート；プロトコル変更 |
| `DashRoot` 個人境界・Pinned Exit は [x000f](x000f-dash-facet-minimal.md) どおり | U1 データ駆動；CJK／IME（→ T2） |

## 完了条件

1. XR: Dash 板が `SpatialLensGpu`＋`uber_sdf` で描画され、CPU タブラベルと二重にならない。
2. Desktop: 背景ディム／Chrome／ヒットが退行しない（fullscreen overlay 維持）。
3. Chrome 操作（タブ切替・Close・Pinned Exit）が従来どおり動く。
4. 新規クレートなし。`spatial_lens` / `system_ui` 関連テスト Green。
5. roadmap 目次・feel 「次の着手」更新。

## 実装メモ（境界）

```text
LayoutTree / SharedDashLogic  （定義・ヒット・状態 SSoT — 触りすぎない）
        ↓ pack（dirty 時）
GpuPanelHeader + GpuUiPrim[]  （Shell / Chrome / 必要なら Card 空き）
        ↓
SpatialLensGpu / uber_sdf.wgsl （XR 本線；Desktop Dash は U0.1）
```

- **置く:** `dash_board` pack、XR `SpatialLensGpu`、`DashState.plate_chrome`。
- **置かない:** Desktop のワールド板＋画面 overlay 二系統、Facet グリッド、taffy、本文全 prim 化。

## 後続（未発行・本スライス外）

| 候補 | 内容 |
|:---|:---|
| **U0.1** | 公式 Facet 本文を Card／Control／Field／MsdfGlyph へ段階移行（CPU content blit 縮小） |
| **U1** | 公式 UI データ駆動（コードハードコード剥がし） |
| **T2 / E2-1** | CJK オンデマンド atlas ＋ XR IME（人承認） |

## 触ったファイル

- `client/app/src/spatial_lens/dash_board.rs` — `OfficialDashView` / `pack_official_dash`
- `client/app/src/spatial_lens/mod.rs` — re-export
- `client/app/src/system_ui/{dash,mod}.rs` — `official_dash_view`・`plate_chrome`（XR のみ）
- `client/app/src/surface/desktop/mod.rs` — Dash は fullscreen overlay 維持（板非載）
- `client/app/src/surface/xr/{wgpu_session,input}.rs` — `SpatialLensGpu`＋`QUAD_*`＝`DASH_*`
- [feel_edit_experience.md](../feel_edit_experience.md) / [README.md](../README.md)

← [目次](../README.md)
