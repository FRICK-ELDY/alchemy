# x0053 — XR ↔ Desktop ホットスイッチ

> **ID:** `x0053` · **slug:** `xr-desktop-hotswitch` · **status:** `done` · **area:** `[client, docs]` · **surface:** `[desktop, vr]` · **depends_on:** `[x0052]`  
> **優先度:** 体感（描画一本化後）

目標: Resonite に近い **同一セッション内での VR ↔ Desktop シームレス切替**を Alchemy に入れる。起動し直しや `ALCHEMY_FORCE_DESKTOP` 固定ではなく、HMD の user presence（および明示トグル）で入出力・提示経路を切り替える。

根拠調査: Resonite `Renderite.Unity.Renderer`（2026-10-30）· Wiki `UseVRHotswitching` / F8 · Issues #1373 / #1765

## なぜ入れるか

| 比較 | 挙動 |
|:---|:---|
| **Resonite** | VR 起動後、HMD 脱着で Desktop ↔ VR を切り替え可能（設定 `UseVRHotswitching`）。手動は **F8**（VR 起動時のみ） |
| **VRChat** | 同等のシームレス切替なし（起動モード固定に近い） |
| **Alchemy（本 Slice）** | OpenXR 起動時に XR session スレッド + Desktop companion。F8 / Settings / `XR_EXT_user_presence` で再起動なし切替 |

## 保証する / しない

| 保証する | 保証しない |
|:---|:---|
| XR session を維持したまま Desktop 窓へ present／入力を切替（`!vr_active` 時は empty layers） | Quest standalone / WebXR の同等 UX |
| `XR_EXT_user_presence` があるとき Settings auto ON で自動切替（300ms ヒステリシス） | 全ランタイムで近接センサの完全一致（VD 経路はベストエフォート） |
| 明示トグル **F8**（Desktop companion） | センサ永久 false 時の万能救済（grace 後 auto 抑止・F8 は残す） |
| READY 前は Desktop が Intent 所有；READY かつ `vr_active` のときだけ XR。切替エッジで Intent ゼロ化。F8 は READY 後のみ | アバター・ロコモーション種別の完全連続；READY 前の F8 |
| Settings「VR hotswitch auto」ON/OFF・`ALCHEMY_HOTSWITCH=0` | VRC 互換／Resonite 設定 UI の複製 |
| フリップ過多 / presence 未検知で auto 抑止 | 単一 wgpu device での窓+swapchain 共有描画 |
| OpenXR init 失敗 → Desktop Solo 相当継続；Zenoh は Desktop 単一所有 | XR スレッドからの Zenoh clone / rebind |

## 実装メモ

1. **検知（`surface/hotswitch` + XR）**: `XR_EXT_user_presence` → `UserPresenceChangedEXT`。無ければ F8 / Settings のみ
2. **決定**: `PresentationGate`（`vr_active` / `hotswitch_auto` / `manual_epoch` / `auto_suppressed`）
3. **実行**: XR は `wait`/`begin`/`end_empty` で session 維持。Desktop companion が窓へ SceneGpu
4. **入力**: `DesktopRole::owns_input`；境界で Pointer Lock・Grab 解放
5. **描画**: XR / Desktop は別 wgpu device（x0052 SceneGpu 経路は各々）。共有は `SharedState` のみ

## Phase

| Phase | 内容 | 状態 |
|:---|:---|:---|
| 0 | 本 Slice（調査・保証たたき）+ 目次 | **done** |
| 1 | presence 読み取り（`XR_EXT_user_presence`） | **done** |
| 2 | 明示トグル（F8）で XR present ↔ Desktop 窓 | **done** |
| 3 | 設定 ON 時の自動切替 + ヒステリシス／壊レセンサ抑止 | **done** |
| 4 | client.md 保証表・手動確認手順 | **done** |

## 完了条件

1. SteamVR + 対応 HMD で、脱着により Desktop ↔ VR が再起動なしで切り替わる（設定 ON・EXT 対応時）— **運用者目視**
2. F8 でも同等に切り替わる
3. 設定 OFF またはセンサ異常ヒューリスティック時は、誤って黒画面／Desktop 固定にならない（auto 抑止・F8 有効）
4. `cargo test -p client` / clippy 緑

## 手動確認（運用者）

1. SteamVR 起動 → `cargo run -p client`（`ALCHEMY_FORCE_DESKTOP` 無し）
2. Desktop 窓タイトルが `VR active (F8 → Desktop)` であること
3. F8 → Desktop 入力（WASD / Grab）・HMD は黒／empty。再 F8 で VR 復帰
4. Settings →「VR hotswitch auto」OFF で脱着しても自動切替しないこと
5. `ALCHEMY_HOTSWITCH=0` で起動すると auto 初期 OFF

## やらない（本 Slice 外）

- Bevy 移行・CompositionLayerQuad
- VRC 互換レイヤ
- FrooxEngine 相当の汎用入出力フレームワーク全面移植

## 関連

- 正本: [client.md](../../0_doc/architecture/client.md)
- 先行: [x0008](x0008-openxr-dash-exit.md) · [x0052](x0052-xr-vk-loop-gpu-remove.md)
- 外部: [Resonite Controls（F8）](https://wiki.resonite.com/Controls) · [GeneralVRSettings.UseVRHotswitching](https://wiki.resonite.com/Component:GeneralVRSettings)

← [目次](../README.md)
