# x0092 — camera / loco eye → display TRS

> **ID:** `x0092` · **slug:** `eye-display-trs` · **status:** `done` · **area:** `[client, sim]` · **depends_on:** `[x0091]`  
> **優先度:** Zero-Base Spatial 本線（x0091 残レガシー撤廃）

目標: Desktop FPS view／Grab ray／loco aim origin／XR floor 基準の **user_root 位置** を WorldView Curr から **`begin_display_frame` の同一 display snap** へ移す。viewpoint アンカーは引き続き `local_user_root`（FPS）。

根拠: [x0091](x0091-epoch-out-scenegpu-draw.md)「やらない／次」· [alchemy_zero_base_spatial_lifecycle_architecture.md](../alchemy_zero_base_spatial_lifecycle_architecture.md) Step 5

---

## 保証する / しない

| 保証する | 保証しない |
|:---|:---|
| Desktop view／Grab ray／loco aim の eye 位置が display snap（α／PrevCurr） | TPS（肩越しオフセット・オービット） |
| XR `floor_model` 用 `user_root` も同一 display 面 | Live／配信カメラ entity・切替 UI |
| unset `local_user_root` 時は従来どおり default eye | OpenXR HMD pose 自体の変更 |
| wire Curr は受理・色チャンネル・未ミラー時フォールバック | XPBD wire 権威 / Rapier 削除 |
| | Hierarchy／Lens 本配線・wire pose ダブルバッファ |

境界メモ（実装しないが将来の継ぎ目）: viewpoint アンカーは現状 `local_user_root` 固定。TPS／Live は **display TRS を取ったあとの合成／選択** として別 Slice。本 Slice に `CameraMode` は置かない。

---

## 設計（要点）

```
begin_display_frame (既存)
        │
        ├─► Grab OBB / coin Y / GPU out     ← x0091
        │
        └─► user_root_pose = display_pose_for(local_user_root, wire)
              ├─ Desktop view / Grab ray / loco aim
              └─ XR floor_model translation
```

- 既存 [`SharedState::display_pose_for`](../../client/app/src/scene.rs) を再利用（新規カメラシステムなし）。
- Desktop `eye_and_look` の Curr 直読みを廃止し、Shared ヘルパに一本化。
- XR の eye は OpenXR HMD のまま；ステージ基準の `user_root` だけ display 化。

---

## Phase

| Phase | 内容 | 状態 |
|:---|:---|:---|
| 0 | Slice + roadmap 目次 | **done** |
| 1 | Shared `user_root_pose` + Desktop / XR 配線 | **done** |
| 2 | 単体（lerp 面 ≠ Curr）+ `cargo test -p client` | **done** |
| 3 | docs（x0091 次・アーキ残債）完了条件緑 | **done** |

---

## 完了条件

1. `cargo test -p client` 緑（`resolve_user_root_display_pose`：lerp ≠ Curr／unset default／欠落フォールバック；XR freeze→eye 順）
2. Desktop eye／Grab ray／loco が Curr 直読みしないこと（`user_root_pose`／frame cache）
3. XR `floor_model` の `user_root` が display ヘルパ経由（`begin_display_frame` 先行）
4. host wire / Rapier 権威を変えていないこと
5. TPS／Live／`CameraMode` を追加していないこと

## やらない（本 Slice）／次

- TPS カメラ合成・Live／配信 viewpoint 切替（別 Slice）
- XPBD を wire 権威にする / Rapier 削除
- Hierarchy UI / Spatial Lens 本配線
- wire pose ダブルバッファ置換（improvement P2-6 残り）

## 関連

- [x0091-epoch-out-scenegpu-draw.md](x0091-epoch-out-scenegpu-draw.md)
- [x0090-client-epoch-scenegpu-bind.md](x0090-client-epoch-scenegpu-bind.md)
- [x008e-zero-base-spatial-lifecycle.md](x008e-zero-base-spatial-lifecycle.md)

← [目次](../README.md)
