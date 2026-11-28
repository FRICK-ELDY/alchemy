# x00ea — Primary レーザー表示ゲート（L0）

> **ID:** `x00ea` · **slug:** `laser-primary-visibility` · **status:** `done` · **area:** `[client]` · **surface:** `[vr]` · **depends_on:** `[x000e, x00bd]`  
> **優先度:** 体験本線 **L0**（Feel / Edit）  
> **体感 1 文:** トリガーを軽く引くまで XR ビームが消える（常時ビームが消える）。

根拠: [feel_edit_experience.md](../feel_edit_experience.md) §4.2 L0

---

## 保証する / しない（本スライス）

| 保証する | 保証しない |
|:---|:---|
| `InteractionLaserConfig.laser_show_primary_min`（既定 **0.1**） | Settings UI 露出・永続化（→ L1） |
| 照準ハンドの Primary（trigger analog）がしきい値未満なら XR ビーム非描画 | Grab/UI 二値しきい値（現行 ≈0.7）の変更 |
| しきい値以上でビーム描画（既存 `laser_model` 経路） | ShowInDesktop・ベジェ描画（→ L3） |
| 純関数／単体テストでゲート判定 | analog Primary 未バインド（例: `khr/simple_controller`）でのビーム表示。照準手は `locate_ui_aim`／`owner_aim` と同一（両手同時ビームは非保証） |

## 完了条件

1. `primary < laser_show_primary_min` → XR `draw_laser` しない。
2. `GRAB_SQUEEZE_THRESHOLD`（≈0.7）および Select/Grab 配線は不変。
3. `laser_show_primary_min` の単体テスト Green。

## 触ったファイル

- `client/app/src/spatial_lens/laser.rs` — `laser_show_primary_min` + `beam_visible` + テスト
- `client/app/src/surface/xr/wgpu_session.rs` — 照準ハンド Primary で `laser_draw` ゲート
- [feel_edit_experience.md](../feel_edit_experience.md) / [README.md](../README.md) / [scene_store_sim_arena.md](../scene_store_sim_arena.md) — 次着手更新

← [目次](../README.md)
