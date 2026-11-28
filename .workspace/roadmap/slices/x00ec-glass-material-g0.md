# x00ec — G0 軽量ガラス語彙（ブラーなし）＋板経路布石

> **ID:** `x00ec` · **slug:** `glass-material-g0` · **status:** `done` · **area:** `[client, docs]` · **depends_on:** `[x00ba]`  
> **優先度:** 体験本線 **G0**（Glass）  
> **体感 1 文:** 公式 UI 板がダークスモーク半透明＋縁 soft glow＋内側グラデでガラスっぽく見え、Desktop/XR（将来 Web）で同じ色語彙になる。

根拠: [feel_edit_experience.md](../feel_edit_experience.md) §4.4 G0 · Lens 正本 §5 危険②

---

## 保証する / しない（本スライス）

| 保証する | 保証しない |
|:---|:---|
| `spatial_lens::glass` 役割トークン（Shell/Chrome/Card/Control/Field/Accent） | 背景ブラー / grab-pass（→ G1） |
| `uber_sdf.wgsl` RoundedBox の内側縦グラデ＋`payload.z` glow＋LOD2=`LOD2_SHELL`（リテラル同期テスト） | 公式 Dash 全コンテンツの板移行完了（→ U0） |
| Shell を `shell_rect_inset`（glow_pad）で板内インセットし clip 内に soft glow | Facet 並べ替え・CustomUIX・MSDF |
| Personal Lens / demo pack がトークン＋`shell_payload` / `card_payload` / `control_payload` を使う。active glow＝hover のみ | |
| CPU Dash 過渡色（`BG`/`BAR`/`BTN`/…）が同じ RGB/α を参照 | 角丸・glow を CPU Canvas で本保証 |
| Desktop / XR / 将来 Web で同一トークン＋同一 WGSL（分岐マテリアルなし） | CSS `backdrop-filter` |

## 完了条件

1. `cargo test -p client spatial_lens` Green（naga・glass・extract）
2. `uber_sdf.wgsl` が `payload.z` glow と LOD2 ShellFill 系を含む
3. 新規クレートなし
4. roadmap 目次・feel 「次の着手」更新

## 触ったファイル

- `client/app/src/spatial_lens/glass.rs` — トークン SSoT
- `client/app/src/spatial_lens/shaders/uber_sdf.wgsl` — グラデ / glow / LOD2
- `client/app/src/spatial_lens/extract.rs` — Lens pack をトークンへ
- `client/app/src/spatial_lens/prim.rs` — `pack_rgba8` を `const fn`
- `client/app/src/spatial_lens/{mod,gpu}.rs` — re-export / 契約テスト
- `client/app/src/system_ui/dash.rs` — CPU 過渡色を glass 参照
- [feel_edit_experience.md](../feel_edit_experience.md) / [README.md](../README.md)

← [目次](../README.md)
