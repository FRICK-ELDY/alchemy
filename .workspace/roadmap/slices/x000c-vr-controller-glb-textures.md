# x000c — コントローラ GLB baseColor テクスチャ

> **ID:** `x000c` · **slug:** `vr-controller-glb-textures` · **status:** done · **area:** `[client]` · **surface:** `[vr]` · **depends_on:** `[x000b]`

目標: `XR_EXT_render_model` が返すコントローラ GLB に埋め込まれた **baseColor** テクスチャがあれば読み、投影スワップチェーン上のコントローラ mesh に貼る。欠如・失敗時は x000b のフラット色へフォールバックする。

正本: [client.md](../../0_doc/architecture/client.md)

## 設計方針

| 層 | 役割 |
|:---|:---|
| client mesh | flatten 時に TEXCOORD_0・任意 baseColor RGBA・baseColorFactor を抽出（汎用。コントローラ固有名なし） |
| render_model | OpenXR から GLB を取得し textured mesh を保持。貼り方は知らない |
| Vulkan multiview | UV 頂点 + sampled image/sampler。欠如時はフラット色。床／ダッシュは現状維持 |
| 資産 | ランタイム提供 GLB の埋め込み画像のみ。SteamVR パス直読みはしない |

| Phase | 内容 | 状態 |
|:---|:---|:---|
| 0 | Slice 文書 + 目次 + client.md / x000b 参照 | done |
| 1 | `mesh.rs`: `CpuSubmesh` / textured flatten（Coin `CpuMesh` は無変更） | done |
| 2 | `render_model.rs`: textured mesh 保持（無しでも Ok） | done |
| 3 | `vk_loop.rs`: UV VBO + sampler + FS sample / flat fallback | done |
| 4 | smoke + モジュール保証コメント更新 | done |

## 保証する / しない

| 保証する | 保証しない |
|:---|:---|
| GLB 内に `baseColorTexture` + `TEXCOORD_0` がある primitive は RGBA を Vulkan に上げてサンプル | metallic-roughness / normal / occlusion / emissive |
| テクスチャ欠如・読取失敗時は現行フラット色（x000b）へフォールバック | SteamVR ディスク直読み（animatable → [x000d](x000d-vr-controller-animatable-nodes.md)） |
| `baseColorFactor` をテクスチャに乗算 | 外部 URI テクスチャ、KTX2、mip 品質保証 |
| マルチマテリアルは primitive = 1 サブメッシュ（各 baseColor を別描画） | Coin / Desktop wgpu 経路のマテリアル本格化 |

## やらない（後続）

- PBR（MR / normal / AO / emissive）
- animatable node（トリガー／スティック可動部）→ [x000d](x000d-vr-controller-animatable-nodes.md)
- `XR_EXT_hand_tracking`
- SteamVR `drivers/.../rendermodels` のローカル読み込み
- Coin / Desktop wgpu のテクスチャ本格化

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

- `client/app/src/mesh.rs` — `load_flattened_textured_from_glb` / `CpuSubmesh`（pos+UV+indices+optional RGBA+factor）
- `client/app/src/surface/xr/render_model.rs` — textured mesh 保持
- `client/app/src/surface/xr/vk_loop.rs` — stride 20 VBO、`upload_rgba_texture`、generation 時のみ再 upload
- smoke: `bash .scripts/smoke-openxr.sh`
- 先行: [x000b-vr-controller-render-models.md](x000b-vr-controller-render-models.md)

← [目次](../README.md)
