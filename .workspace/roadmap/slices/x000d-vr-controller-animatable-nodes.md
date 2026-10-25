# x000d — コントローラ animatable node

> **ID:** `x000d` · **slug:** `vr-controller-animatable-nodes` · **status:** done · **area:** `[client]` · **surface:** `[vr]` · **depends_on:** `[x000c]`

目標: `XR_EXT_render_model` の animatable node state を毎フレーム取得し、トリガー／スティック等の可動部を親相対 pose と visibility で投影スワップチェーン上に反映する。VBO は静的のまま、model 行列だけ差し替える。

正本: [client.md](../../0_doc/architecture/client.md)

## 設計方針

| 層 | 役割 |
|:---|:---|
| client mesh | 名前付き runtime 置換ノードを受け取り、rig 階層 + 局所頂点を構築（OpenXR 固有名なし） |
| render_model | asset から可動ノード名を取得。毎フレーム state を poll し node world を合成 |
| Vulkan multiview | サブメッシュごとに `rootPose * nodeWorld`。不可視は draw skip。テクスチャは x000c 維持 |
| 資産 | ランタイム提供 GLB + EXT state のみ |

| Phase | 内容 | 状態 |
|:---|:---|:---|
| 0 | Slice 文書 + 目次 + client.md / x000c 参照 | done |
| 1 | `mesh.rs`: `RigNode` / `load_textured_rigged_from_glb` | done |
| 2 | `render_model.rs`: asset node names + `poll_state` + world 合成 | done |
| 3 | `vk_loop.rs`: 毎フレーム poll + per-node draw | done |
| 4 | smoke + モジュール保証コメント更新 | done |

## 保証する / しない

| 保証する | 保証しない |
|:---|:---|
| SteamVR 2.12+ で `animatableNodeCount>0` のとき、state の pose／visibility を反映 | PBR（MR / normal / AO / emissive） |
| count=0／state 失敗時は静的 mesh（x000c）へフォールバック | スキン／モーフ、Desktop wgpu への anim 移植 |
| 埋め込み baseColor（x000c）経路は維持 | SteamVR ディスク直読み、`XR_EXT_hand_tracking` |

## やらない（後続）

- PBR
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

- `client/app/src/mesh.rs` — `RigNode` / `load_textured_rigged_from_glb` / `CpuSubmesh.node_index`
- `client/app/src/surface/xr/render_model.rs` — asset names + `poll_state` + world 合成
- `client/app/src/surface/xr/vk_loop.rs` — per-node model / visibility draw
- smoke: `bash .scripts/smoke-openxr.sh`
- 先行: [x000c-vr-controller-glb-textures.md](x000c-vr-controller-glb-textures.md)

← [目次](../README.md)
