# x0050 — XR swapchain を wgpu に載せる（P4-1 / 1/3）

> **ID:** `x0050` · **slug:** `xr-swapchain-wgpu` · **status:** `done` · **area:** `[client, docs]` · **depends_on:** `[]`  
> **優先度:** **P4-1**（描画経路統合・第1スライス）

目標: OpenXR が作った Vulkan instance/device を **wgpu-hal 経由で採用**し、acquire した swapchain image を `wgpu::Texture` として clear／present できることを証明する。メッシュ描画の移行はしない。（完了）

根拠: [improvement-plan-2026-10-28.md](../../0_doc/evaluation/improvement-plan-2026-10-28.md) **P4-1** · 正本 [client.md](../../0_doc/architecture/client.md)

## 保証する / しない

| 保証する | 保証しない |
|:---|:---|
| XR Vulkan device と wgpu Device を同一物理 device 上で共有できる | SceneGpu／メッシュ／UI／レーザー／コントローラの描画移行（→ x0051） |
| acquire した swapchain image を wgpu から clear して compositor に渡せる | ash 手書きパイプラインの削除（→ x0052） |
| `ALCHEMY_XR_WGPU=1` のときだけ clear-only 経路。未設定は現行 ash 描画 | multiview シェーダの wgpu 移植（stereo は後続で dual-eye） |
| drop 順（textures → swapchain → frame_stream/wait → session → device）と `unsafe` 根拠コメント | Bevy 導入・新規描画依存の追加（必要なら人承認） |
| フラグ OFF で現行 XR 挙動を壊さない | WSL2 実 HMD / Quest standalone |

## 設計（要点）

1. OpenXR `vulkan_enable2` で instance/device を作成（現行どおり runtime 経由）
2. wgpu-hal: `Instance::from_raw` → `expose_adapter` → `device_from_raw`（XR が所有・drop_callback で非破壊）
3. wgpu: `Instance::from_hal` / `create_adapter_from_hal` / `create_device_from_hal`
4. swapchain `VkImage` → `texture_from_raw`（External）→ `create_texture_from_hal`
5. モジュール: `client/app/src/surface/xr/wgpu_interop.rs` + `wgpu_clear.rs`

## Phase

| Phase | 内容 | 状態 |
|:---|:---|:---|
| 0 | Slice + 目次 | done |
| 1 | wgpu_interop（device 橋渡し + texture wrap） | done |
| 2 | `ALCHEMY_XR_WGPU=1` clear-only 経路 | done |
| 3 | client.md 保証表 + 単体／CI 緑 | done |

## 完了条件

1. フラグ ON で HMD（または SteamVR null）に wgpu clear 色が出る
2. フラグ OFF で現行 XR（ash）挙動を壊さない
3. drop 順と各 `unsafe` に安全性コメントがある
4. `cargo test -p client` / 既存 CI 緑

## やらない

- SceneGpu へのワールド描画寄せ
- ash パイプライン削除
- multiview 化

## 手動確認

```powershell
# スタック + host 起動済み、SteamVR 起動
cd client
$env:ALCHEMY_XR_WGPU = "1"
# ALCHEMY_FORCE_DESKTOP は付けない
cargo run -p client
# HMD: マゼンタ寄り clear（ash の暗い teal ではない）
```

完了条件の HMD マゼンタは **運用者の目視**が正本（CI は unit / clippy のみ）。黒画面や validation が出たら layout／同期を疑う。

← [目次](../README.md) · 次: [x0051](x0051-xr-scene-gpu-unify.md)
