# x00ed — T1 Static MSDF（ASCII / 数字 / アイコン）

> **ID:** `x00ed` · **slug:** `static-msdf-t1` · **status:** `done` · **area:** `[client, docs]` · **depends_on:** `[x00ba, x00bc]`  
> **優先度:** 体験本線 **T1**（Text）  
> **体感 1 文:** Spatial Lens の英語ラベルと数値桁が、常駐 Static MSDF アトラス経由で斜めからでも読める文字になる。

根拠: [feel_edit_experience.md](../feel_edit_experience.md) §4.3 T1 · Lens 正本 §4.2 / §5 危険① Tier 1

---

## 保証する / しない（本スライス）

| 保証する | 保証しない |
|:---|:---|
| Tier1 Static Atlas 512² RGBA8・ASCII printable + 基本アイコン数個・UV ルックアップ | 真の msdfgen 多チャンネル品質・フォントファイル依存 |
| `MsdfGlyph` を `uber_sdf` で描画（payload `[u0,v0,u1,v1]`） | CJK / 絵文字 / LRU 動的アトラス（→ T2） |
| DirectNumber の `0-9` / `-` / `.` をアトラス UV からサンプル（Zero-String 維持） | ライブ数値の `format!` / String カタログ |
| Lens タイトル／Inspector 行の短い ASCII ラベルを MsdfGlyph で出す | 文言カタログ・ロケール解決 |
| naga 検証＋`cargo test -p client spatial_lens` Green | Dash CPU 5×7 置換、Dash 全板移行（→ U0） |

## 完了条件

1. `cargo test -p client spatial_lens` Green（naga・msdf bake・extract）
2. `uber_sdf.wgsl` が atlas bind + `sample_msdf` + MsdfGlyph / DirectNumber アトラス桁を含む
3. 新規クレートなし
4. roadmap 目次・feel 「次の着手」更新

## 触ったファイル

- `client/app/src/spatial_lens/msdf.rs` — Static atlas bake / UV / push
- `client/app/src/spatial_lens/shaders/uber_sdf.wgsl` — median MSDF sample
- `client/app/src/spatial_lens/gpu.rs` — atlas texture + sampler bind
- `client/app/src/spatial_lens/extract.rs` — Lens / demo MsdfGlyph 配線
- `client/app/src/spatial_lens/{mod,prim}.rs` — re-export / 契約コメント
- [feel_edit_experience.md](../feel_edit_experience.md) / [README.md](../README.md)

← [目次](../README.md)
