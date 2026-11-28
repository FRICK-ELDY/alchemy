# x00f5 — XR ビーム角度変調スムージング（L2）

> **ID:** `x00f5` · **slug:** `laser-xr-smooth-l2` · **status:** `done` · **area:** `[client, docs]` · **surface:** `[vr]` · **depends_on:** `[x00ea, x00ee]`  
> **優先度:** 体験本線 **L2**（Laser）  
> **体感 1 文:** XR でコントローラを振るとビームが Settings の Smooth/Modulate どおり滑らかに追従する。

根拠: [feel_edit_experience.md](../feel_edit_experience.md) §4.2 L2 · [x00ee](x00ee-laser-settings-l1.md)

---

## 保証する / しない（本スライス）

| 保証する | 保証しない |
|:---|:---|
| XR ワールド／UI レーザーの向きに L1 の `laser_smooth_speed` / `modulate_*` を本配線 | Desktop 経路の新規スムージング（既存 Desktop Lens 挙動は非対象のまま） |
| Settings / `laser.json` の既存キーを読み、XR セッション側へ反映（描画＋同一レイの Dash／Grab／Context ヒット） | L3（ベジェ・ShowInDesktop ビーム描画・Cursor 倍率） |
| Context open／re-anchor は当該手を `snap_direction` してから **smooth 向きで freeze**（leave／draw と同一レイ） | XR Stickiness / press-lock 配線 |
| トラッキング再取得・hotswitch、および連続 N フレーム `!should_render` 後の復帰で `snap_direction`（短い空フレームは smooth 維持） | 新規クレート；プロトコル変更 |
| Primary 表示ゲート（L0）・Grab/UI 二値しきい値は不変；既存 `update_smoothed_direction` 再利用 | |
| Settings **Reset defaults**（`InteractionLaserConfig::default` へ復帰・永続） | |
| Alchemy Laser Default（後続 Empiric: Smooth/Start/Stick **0**・`≤0` raw。x00f5 当時は 5/2°/45°/…。Resonite 出荷値は未ピン） | 既存 `laser.json` の自動移行（明示 Reset まで旧値維持） |

## 完了条件

1. XR 照準レイの向きが `laser_smooth_speed` / `modulate_*` で角度変調スムージングされる。
2. スムーズ向きが `laser_model`・長さ計算・Dash pointer・world grab aim で一致する。
3. `beam_visible`（L0）と `GRAB_SQUEEZE_THRESHOLD` は不変。
4. 関連単体テスト Green・roadmap / feel 更新。

## 触ったファイル

- `client/app/src/surface/xr/input.rs` — `aim_orient_from_smoothed_dir`
- `client/app/src/surface/xr/wgpu_session.rs` — 左右 `HandLaserState`・smooth 適用
- `client/app/src/spatial_lens/laser.rs` — smooth 本線・Default チューニング・Reset・modulate テスト
- `client/app/src/system_ui/{dash,layout,facet,mod}.rs` — Settings Laser hint・Reset defaults
- [feel_edit_experience.md](../feel_edit_experience.md) / [README.md](../README.md) / [alchemy_zero_base_spatial_ui_lens_uix_architecture.md](../alchemy_zero_base_spatial_ui_lens_uix_architecture.md)

← [目次](../README.md)
