# x000e — VR UI レーザーポインタ（簡易）

> **ID:** `x000e` · **slug:** `vr-ui-laser-pointer` · **status:** done · **area:** `[client]` · **surface:** `[vr]` · **depends_on:** `[x0008, x000b]`

目標: ダッシュ開中にコントローラ **aim** から薄いレーザービームを投影描画し、既存の Select＋レイヒットで Exit 等を狙いやすくする。Exit クリック配線自体は x0008 のまま。（完了）

正本: [client.md](../../0_doc/architecture/client.md)

## 設計方針

| 層 | 役割 |
|:---|:---|
| `input.rs` | ヒット距離 `t`・`laser_model`（単位 -Z ボックス）の純関数 |
| `vk_loop` | ダッシュ open 時のみ aim（右→左）でビーム描画。Select クリックと aim を共有 |
| mesh パイプライン | `MESH_MODE_FLAT` + `push.factor`（シアン）。コントローラ fallback 色は据え置き |
| system_ui | OpenXR / レイを知らない（ピクセル `on_click` のみ） |

| Phase | 内容 | 状態 |
|:---|:---|:---|
| 0 | Slice 文書 + 目次 | done |
| 1 | `input.rs` 距離 / model + tests | done |
| 2 | `vk_loop` FLAT mode・laser mesh・aim 共有 | done |
| 3 | docs + smoke | done |

## VR バインド

| 入力 | 役割 | 本 Slice |
|:---|:---|:---|
| Menu | ダッシュ開閉 | 既存（開中のみビーム） |
| Select + aim | ダッシュ UI クリック / Exit | 既存（可視化のみ追加） |
| aim pose | ビーム原点・向き | 実装 |

## 保証する / しない

| 保証する | 保証しない |
|:---|:---|
| ダッシュ open 時、右→左 aim の 1 本レーザーを投影に描画 | 両ハンド同時・ダッシュ閉時のワールドポインタ |
| クアッドヒット時はヒット距離、ミス時は固定最大長 | ホバーハイライト・カーソル描画 |
| 既存 Select→Exit 経路を壊さない | CompositionLayerQuad より手前のビーム先端（簡易で許容） |
| WSL CI / Desktop 回帰（smoke） | WSL 実 HMD |

## やらない（後続）

- UI ホバーハイライト
- ダッシュ閉時のワールドレーザー / Grab
- system_ui への XR 知識流入

## 起動

```bash
cd auth-server && docker compose up -d && cd ..
cd world-server && docker compose up -d && cd ..
cd assets-server && docker compose up -d && cd ..
zenohd -c zenoh-router/config.json5
cd client
cargo run -p host-server
# Windows + SteamVR（WSL は HMD 非保証）
cargo run -p client
```

実機:

1. Menu でダッシュ開 → aim からシアンのビームが出ることを確認  
2. Exit タブ → 「client のみ終了」等をビームで狙い Select  
3. ダッシュ閉でビームが消えること

## 実装メモ

- `client/app/src/surface/xr/input.rs` — ヒット距離 / `laser_model` / `LASER_MAX_LENGTH`
- `client/app/src/surface/xr/vk_loop.rs` — laser mesh + `MESH_MODE_FLAT` + aim 共有
- smoke: `bash .scripts/smoke-openxr.sh`
- 先行: [x0008-openxr-dash-exit.md](x0008-openxr-dash-exit.md) · [x000b-vr-controller-render-models.md](x000b-vr-controller-render-models.md)

← [目次](../README.md)
