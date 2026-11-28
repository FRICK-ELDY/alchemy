# x00ee — Settings ↔ InteractionLaserConfig（L1）

> **ID:** `x00ee` · **slug:** `laser-settings-l1` · **status:** `done` · **area:** `[client, docs]` · **surface:** `[vr, desktop]` · **depends_on:** `[x00ea, x00bd]`  
> **優先度:** 体験本線 **L1**（Laser）  
> **体感 1 文:** Settings の Laser を触ると再起動後も残る。XR ではビームの出し方（Primary しきい値）、Desktop Lens では滑らかさ・くっつきが変わる。

根拠: [feel_edit_experience.md](../feel_edit_experience.md) §4.2 L1

---

## 保証する / しない（本スライス）

| 保証する | 保証しない |
|:---|:---|
| Resonite LaserSettings 写像 8 項目を Settings に露出（7 float + ShowInDesktop） | ベジェ描画・Cursor 倍率（→ L3） |
| `laser.json` へのローカル永続化と boot 復元 | 文言カタログ・仮想キーボード編集 |
| Desktop / XR への cfg ライブ反映（値の到着） | XR での smooth / modulate / stickiness **見た目**（→ L2） |
| `laser_show_primary_min` → XR `beam_visible` | Grab/UI 二値しきい値（≈0.7）の変更 |
| Desktop Lens 経路での smooth / stickiness 適用 | Desktop 可視化ビーム描画本体（→ L3；Lens ピッキングは止めない） |
| `show_lasers_in_desktop_mode` フィールド＋UI＋永続 | ShowInDesktop ON 時のビームオーバーレイ（→ L3） |

## 完了条件

1. Settings Laser 節から写像項目を変更できる（float は ±、ShowInDesktop はトグル）。
2. 再起動後も `laser.json` から復元される。
3. Desktop / XR の runtime cfg に値が届く（XR 見た目の smooth/stick は非保証 → L2。XR で効くのは `laser_show_primary_min` → `beam_visible`）。
4. 新規クレートなし・関連テスト Green。
5. roadmap 目次・feel 「次の着手」更新。

## 触ったファイル

- `client/app/src/spatial_lens/laser.rs` — `show_lasers_in_desktop_mode` + clamp/step + 非有限 → Default
- `client/app/src/laser_prefs.rs` — JSON load/save・coalesce 書き込み・spawn 失敗 sync drain・`flush_blocking`
- `client/app/src/system_ui/{dash,layout,mod,facet}.rs` — Settings Laser 節・hint ラベル・レイアウト定数一本化
- `client/app/src/surface/desktop/mod.rs` / `surface/xr/wgpu_session.rs` — cfg 配線・exit flush
- [feel_edit_experience.md](../feel_edit_experience.md) / [README.md](../README.md)

← [目次](../README.md)
