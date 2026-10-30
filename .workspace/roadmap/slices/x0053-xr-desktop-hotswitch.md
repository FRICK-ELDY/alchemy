# x0053 — XR ↔ Desktop ホットスイッチ

> **ID:** `x0053` · **slug:** `xr-desktop-hotswitch` · **status:** `planned` · **area:** `[client, docs]` · **surface:** `[desktop, vr]` · **depends_on:** `[x0052]`  
> **優先度:** 候補（P4 帯・描画一本化後の体感）

目標: Resonite に近い **同一セッション内での VR ↔ Desktop シームレス切替**を Alchemy に入れる。起動し直しや `ALCHEMY_FORCE_DESKTOP` 固定ではなく、HMD の user presence（および明示トグル）で入出力・提示経路を切り替える。

根拠調査: Resonite `Renderite.Unity.Renderer`（2026-10-30）· Wiki `UseVRHotswitching` / F8 · Issues #1373 / #1765

## なぜ入れるか

| 比較 | 挙動 |
|:---|:---|
| **Resonite** | VR 起動後、HMD 脱着で Desktop ↔ VR を切り替え可能（設定 `UseVRHotswitching`）。手動は **F8**（VR 起動時のみ） |
| **VRChat** | 同等のシームレス切替なし（起動モード固定に近い） |
| **Alchemy 現状** | 起動時に OpenXR あり→XR、なし／`ALCHEMY_FORCE_DESKTOP`→Desktop。**実行中の切替は未対応**（x0008 / client.md） |

HMD を外してキーボード作業し、着け直してそのまま VR に戻る、という日常動線を Alchemy でも保証候補にする。

## Resonite で分かったこと（参照実装）

マルチプロセス: Unity レンダラは **presence を報告するだけ**。モード決定は FrooxEngine（非公開）。

| ランタイム経路 | `userPresentInHeadset` の条件 |
|:---|:---|
| **SteamVR** | `OpenVR.System.GetTrackedDeviceActivityLevel(HMD) == UserInteraction` のときだけ true。Idle / UserInteraction_Timeout / Standby / Unknown は **false** |
| **Oculus ネイティブ** | `OVRPlugin.userPresent`（`ovrp_GetUserPresent()`・近接センサ） |

補足:

- OpenVR 文書上の「InUse = Interaction \|\| Timeout」より厳しく、Resonite は **UserInteraction のみ**を present としている
- VR 非アクティブ時、レンダラは `compositor.SuspendRendering(true)` するだけ（切替ロジック本体ではない）
- 近接センサ不良向けに Frooxius は「壊れたセンサを検知して VR に留めるヒューリスティック」があると述べている（#1765）。実装は Renderite リポ外
- Virtual Desktop 等で presence が来ないと hotswitch しない事例あり（#1373）

参照パス（調査時）:

- `Assets/APP/Unity Environment/InputOutputDrivers/SteamVRDriver.cs`（activity level → `userPresentInHeadset` / `OnVR_ActiveChanged`）
- `Assets/APP/Unity Environment/InputOutputDrivers/OculusDriver.cs`（`OVRPlugin.userPresent`）

## Alchemy での保証する / しない（実装時のたたき）

| 保証する（案） | 保証しない（案） |
|:---|:---|
| XR セッションを維持したまま、Desktop（winit）と XR present を切替できる | Quest standalone / WebXR の同等 UX |
| OpenXR（または SteamVR）の **user presence** 相当を毎フレーム読み、設定 ON 時に自動切替 | 全ランタイムで近接センサの完全一致（ランタイム差・VD 経路はベストエフォート） |
| 明示トグル（Desktop キー。Resonite の F8 相当） | センサ永久 false 時の万能救済（ヒステリシスは入れるが、壊れた HW は手動トグル／設定 OFF） |
| 切替時に look / Grab / Intent が破綻しない（解放または安全な既定へ） | アバター・ロコモーション種別の完全連続（必要なら Desktop 用に再マップ） |
| Settings で hotswitch ON/OFF | VRC 互換／Resonite 設定 UI の複製 |

## 設計メモ（未実装・層分け）

1. **検知（surface/xr）**: OpenXR `XR_EXT_user_presence` 等があればそれを正本。無い場合はランタイム固有フォールバックを surface に閉じる（system_ui / host は知らない）
2. **決定（client boot / surface）**: presence + 設定 + 明示トグル → `vr_active`。壊れたセンサ用に「一度も true にならない／フリップ異常」なら自動切替を抑止
3. **実行**: XR は submit 停止または session を活かしたまま Desktop 窓を前面。再装着で submit 再開。**再起動しない**
4. **入力**: Desktop 時は winit／XR 時は OpenXR。切替境界で Pointer Lock・Grab を解放
5. **描画**: x0052 以降の SceneGpu 単一経路を前提に、ターゲット（swapchain vs 窓）だけ切る

## Phase（案）

| Phase | 内容 | 状態 |
|:---|:---|:---|
| 0 | 本 Slice（調査・保証たたき）+ 目次 | **active（文書のみ）** |
| 1 | presence 読み取り API 調査（SteamVR OpenXR / 実機） | planned |
| 2 | 明示トグル（キー）で XR present ↔ Desktop 窓 | planned |
| 3 | 設定 ON 時の自動切替 + ヒステリシス／壊レセンサ抑止 | planned |
| 4 | client.md 保証表・手動確認手順 | planned |

## 完了条件（実装時）

1. SteamVR + 対応 HMD で、脱着により Desktop ↔ VR が再起動なしで切り替わる（設定 ON）
2. 明示トグルでも同等に切り替わる
3. 設定 OFF またはセンサ異常ヒューリスティック時は、誤って黒画面／Desktop 固定にならない
4. `cargo test -p client` / clippy 緑。HMD 目視は運用者正本

## やらない（本 Slice 外）

- Bevy 移行・CompositionLayerQuad
- VRC 互換レイヤ
- FrooxEngine 相当の汎用入出力フレームワーク全面移植

## 関連

- 正本: [client.md](../../0_doc/architecture/client.md)（現状は起動時分岐のみ）
- 先行: [x0008](x0008-openxr-dash-exit.md) · [x0052](x0052-xr-vk-loop-gpu-remove.md)
- 外部: [Resonite Controls（F8）](https://wiki.resonite.com/Controls) · [GeneralVRSettings.UseVRHotswitching](https://wiki.resonite.com/Component:GeneralVRSettings)

← [目次](../README.md)
