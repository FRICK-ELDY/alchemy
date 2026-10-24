# client

> **正本**: [`client/`](../../../client/)（Cargo workspace）  
> **ステータス**: Slice C-B（assets GLB + wgpu mesh。Zenoh frame/input は維持）

## レイアウト

| パス | 役割 |
|:---|:---|
| `client/Cargo.toml` | workspace 根（将来 submodule の根） |
| `client/app` | 画面プロセス（パッケージ名 `client`） |
| `client/host-server` | 裏プロセスの計算ホスト |
| `client/fixtures/` | C-B 用 GLB（Coin_PolygonalMind、CC0） |
| `../protocol` | 共有ワイヤ契約（workspace 外） |

## 保証すること / しないこと

| 保証する | 保証しない |
|:---|:---|
| Zenoh client として frame Subscribe / input Publish | ルーター機能（他人の通信の中継は禁止） |
| auth login → Bearer 付きで Room Token 取得し InputEvent に添付 | 本格ログイン UI（シードアカウント） |
| assets Bearer download（失敗時は fixture）→ BLAKE3 照合 → gltf → wgpu 1 メッシュ | LOD / KTX2 / マテリアル本格 / スキン |
| | OSC / Dual Editor / launcher 本実装 |

## Slice C-B アセット

- モデル: Coin_PolygonalMind（Polygonal Mind / medieval-fair、CC0）
- BLAKE3: `fa24848fa7b971718ce6e8dc9bd2d211808fd64e4ddf555feb66d4f82ee1e72a`
- 詳細: [`client/fixtures/README.md`](../../../client/fixtures/README.md)

環境変数: `ASSETS_URL`（既定 `:4004`）、任意 `ALCHEMY_BLOB_BLAKE3`。

## 関連

- ワイヤ契約: [protocol.md](protocol.md)
- 転送: [zenoh-router.md](zenoh-router.md)
- Room Token: [world-server.md](world-server.md)
- Blob CAS: [assets-server.md](assets-server.md)
