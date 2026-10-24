# x0004 — Coin GLB を wgpu 表示（完了）

> **ID:** `x0004` · **旧称:** Slice C-B · **slug:** `coin-glb-wgpu` · **status:** done · **area:** `[client, assets]` · **depends_on:** `[x0003]`

目標: assets-server から Coin_PolygonalMind（CC0）を取得し、BLAKE3 照合後に client で wgpu 表示する。  
正本: [client.md](../../0_doc/architecture/client.md) · fixture: [`client/fixtures/`](../../../client/fixtures/)

| Phase | 内容 | 状態 |
|:---|:---|:---|
| 0 | Coin fixture 同梱 + 出典 / BLAKE3 | done |
| 1 | auth download → BLAKE3 → gltf → wgpu mesh | done |
| 2 | smoke-assets-glb + docs | done |

## 保証する / しない

| 保証する | 保証しない |
|:---|:---|
| 1 GLB（Coin）の download / fixture フォールバック | LOD / KTX2 / P2P キャッシュ |
| BLAKE3 不一致時は描画しない | マテリアル・スキン・アニメ |
| wgpu で 1 メッシュ表示 | host-server へのメッシュ同期 |

## 起動

```bash
cd auth-server && docker compose up -d && cd ..
cd assets-server && docker compose up -d && cd ..
bash .scripts/smoke-assets-glb.sh
# zenohd + world + host-server 後:
cd client && cargo run -p client
```

← [目次](../README.md)
