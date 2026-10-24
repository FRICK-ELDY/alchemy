# client

> **正本**: [`client/`](../../../client/)（Cargo workspace）  
> **ステータス**: Slice C-C（SceneManifest + delta RenderFrame + Coin mesh）

## レイアウト

| パス | 役割 |
|:---|:---|
| `client/Cargo.toml` | workspace 根（将来 submodule の根） |
| `client/app` | 画面プロセス（パッケージ名 `client`） |
| `client/host-server` | 裏プロセスの計算ホスト |
| `client/fixtures/` | Coin_PolygonalMind GLB（CC0） |
| `../protocol` | 共有ワイヤ契約（workspace 外） |

## 保証すること / しないこと

| 保証する | 保証しない |
|:---|:---|
| Zenoh: Manifest Subscribe / frame デルタ Subscribe / input Publish | ルーター機能（他人の通信の中継は禁止） |
| Manifest の blake3 → assets download（失敗時 fixture）→ BLAKE3 → wgpu | LOD / KTX2 / マテリアル本格 / スキン |
| ローカル entity 表への upsert/remove 適用、mesh が entity 1 pose に追従 | 署名付き Manifest / world 台帳 |
| auth login → Bearer 付き Room Token → InputEvent | OSC / Dual Editor / launcher 本実装 |

## チャネル分離（C-C）

| キー | 頻度 | 用途 |
|:---|:---|:---|
| `…/manifest` | 低（host は 5s 再送で late joiner 補助） | `entity_id → blake3` |
| `…/frame` | 高（動いたものだけ） | pose upserts / removes |
| `…/input` | 入力 | Room Token 付き InputEvent |

## アセット

- モデル: Coin_PolygonalMind（Polygonal Mind / medieval-fair、CC0）
- BLAKE3: `fa24848fa7b971718ce6e8dc9bd2d211808fd64e4ddf555feb66d4f82ee1e72a`
- 詳細: [`client/fixtures/README.md`](../../../client/fixtures/README.md)

環境変数: `ASSETS_URL`（既定 `:4004`）、任意 `ALCHEMY_BLOB_BLAKE3`（Manifest 待ちタイムアウト時のフォールバック）。

## 関連

- ワイヤ契約: [protocol.md](protocol.md)
- 転送: [zenoh-router.md](zenoh-router.md)
- Room Token: [world-server.md](world-server.md)
- Blob CAS: [assets-server.md](assets-server.md)
