# x000b — OpenXR Render Model でコントローラ表示

> **ID:** `x000b` · **slug:** `vr-controller-render-models` · **status:** done · **area:** `[client]` · **surface:** `[vr]` · **depends_on:** `[x000a]`

目標: SteamVR フォルダの資産を同梱せず、`XR_EXT_render_model` / `XR_EXT_interaction_render_model` でランタイムから GLB を取得し、投影スワップチェーンにコントローラを描画する。

正本: [client.md](../../0_doc/architecture/client.md)

## 設計方針

| 層 | 役割 |
|:---|:---|
| OpenXR EXT | ID 列挙 → createRenderModel → GLB asset → render model space |
| client mesh | ノード変換焼き込み flatten（マテリアル非保証） |
| Vulkan multiview | clear + depth + フラット色 mesh。床／ダッシュ quad は現状維持 |
| 資産 | ランタイム提供のみ。SteamVR `rendermodels` パス直読みはしない |

| Phase | 内容 | 状態 |
|:---|:---|:---|
| 0 | Slice 文書 + 目次 | done |
| 1 | openxr 0.22 + EXT 有効化（欠如時 warn 続行） | done |
| 2 | render_model ラッパ + GLB flatten | done |
| 3 | multiview depth + mesh 描画 | done |
| 4 | smoke + docs | done |

## 保証する / しない

| 保証する | 保証しない |
|:---|:---|
| SteamVR（Windows）2.12+ で EXT 利用時、実機コントローラ GLB をルート pose で表示 | SteamVR フォルダからの資産コピー／同梱 |
| 拡張欠如時も既存ダッシュ／ロコは動作 | ボタン等 animatable node のアニメ |
| ランタイム提供 GLB のみ（再配布なし） | テクスチャ／PBR、手トラッキング、Coin mesh in XR（テクスチャは [x000c](x000c-vr-controller-glb-textures.md)） |

## やらない（後続）

- animatable node（トリガー／スティック可動部）
- コントローラテクスチャ／マテリアル忠実度 → [x000c](x000c-vr-controller-glb-textures.md)
- `XR_EXT_hand_tracking`
- SteamVR `drivers/.../rendermodels` のローカル読み込み

## 起動

```bash
cd auth-server && docker compose up -d && cd ..
cd world-server && docker compose up -d && cd ..
cd assets-server && docker compose up -d && cd ..
zenohd -c zenoh-router/config.json5
cd client
cargo run -p host-server
# Windows + SteamVR 2.12+（WSL は HMD 非保証）
cargo run -p client
```

## 実装メモ

- `client` — `openxr = "0.22"`
- `client/app/src/surface/xr/render_model.rs` — EXT ラッパ
- `client/app/src/surface/xr/vk_loop.rs` — 拡張有効化 + mesh 描画
- `client/app/src/mesh.rs` — `load_flattened_mesh_from_glb`
- smoke: `bash .scripts/smoke-openxr.sh`

← [目次](../README.md)
