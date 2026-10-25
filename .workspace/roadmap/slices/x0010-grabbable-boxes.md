# x0010 — Grabbable ボックス最小（ワールド構成）

> **ID:** `x0010` · **slug:** `grabbable-boxes` · **status:** done · **area:** `[host, client, sim]` · **surface:** `[desktop, vr]` · **depends_on:** `[x0006, x000e]`

目標: [Resonite Grabbable](https://wiki.resonite.com/Component:Grabbable) が前提にする最小条件だけを切る — **コライダー付き立方体 ×9（3×3）**、host 側 SoA の薄い `grabbable` 印、client の色付き描画、ダッシュ閉時ワールドレーザーの **レイヒット可視化**。Grab 押下での追従は **x0011**。（完了）

正本: [client.md](../../0_doc/architecture/client.md) · [protocol.md](../../0_doc/architecture/protocol.md)

## 設計方針

| 層 | 役割 |
|:---|:---|
| host `PhysicsWorld` | Rapier 動的 cuboid ×9 + コライダー。SoA `grabbable[]`（本 Slice は全 true） |
| `RenderFrame` | entity `10..18` の pose + rgba デルタ（手続きボックス。Manifest blake3 なし） |
| client 描画 | 単位立方体 × scale。Desktop wgpu / XR mesh FLAT |
| surface XR | ダッシュ閉時 aim レーザーをボックス AABB に当てて長さ・ハイライト |
| `Grabbable` | **host 内フラグのみ**。ワイヤ Component スキーマは載せない |

| Phase | 内容 | 状態 |
|:---|:---|:---|
| 0 | Slice 文書 + 目次 | done |
| 1 | host: 3×3 cuboid SoA + frame デルタ | done |
| 2 | client: 色付き立方体描画（Desktop + XR） | done |
| 3 | XR: ダッシュ閉時ワールドレーザー + AABB ヒット | done |
| 4 | smoke + client.md / README | done |

## 保証する / しない

| 保証する | 保証しない |
|:---|:---|
| 動的 cuboid ×9（等間隔 3×3）+ コライダーが Rapier 上に存在する | Slot 階層 / 汎用 Component ランタイム |
| host SoA `grabbable`（全 true） | Steal / Scalable / Receivable 等 Resonite 互換フィールド |
| frame で entity 10..18 の pose+rgba が届く | Manifest への blake3 必須化 |
| Desktop / XR で色付き立方体として見える | GLB マテリアル / LOD |
| ダッシュ閉時 aim レーザーがボックスに当たり長さが縮む（ヒット可視化） | Grab 押下での kinematic 追従・放下（→ x0011） |
| Coin（entity 1）維持 | Desktop Grab / マウスルック / proto `grab_entity_id` |

## やらない（後続）

- Grab hold（grip → transform 追従）→ **x0011**
- 汎用 Slot / Component / ProtoFlux 相当
- WorldRoot UI・Facet 並べ替え
- Formula / Dual Editor

## 起動

```bash
cd auth-server && docker compose up -d && cd ..
cd world-server && docker compose up -d && cd ..
cd assets-server && docker compose up -d && cd ..
zenohd -c zenoh-router/config.json5
cd client
cargo run -p host-server
# Desktop
ALCHEMY_FORCE_DESKTOP=1 cargo run -p client
# Windows + SteamVR（WSL は HMD 非保証）
cargo run -p client
```

確認:

1. 床上に色の違う立方体が 3×3 で並ぶ  
2. XR: ダッシュ閉 → aim レーザーがボックスに当たるとビームが短くなる／ハイライト  
3. Grab しても（本 Slice では）動かない  

## 実装メモ

- `client/host-server/src/sim.rs` — boxes SoA（handles / grabbable / half_extent）
- `client/host-server/src/main.rs` — entity 10..18 デルタ
- `client/app/src/scene.rs` — Desktop 立方体描画
- `client/app/src/surface/xr/input.rs` — AABB レイヒット
- `client/app/src/surface/xr/vk_loop.rs` — ボックス + ダッシュ閉レーザー
- smoke: `bash .scripts/smoke-rapier-host.sh`（boxes ready 行）
- 後続: Grab hold → x0011

← [目次](../README.md)
