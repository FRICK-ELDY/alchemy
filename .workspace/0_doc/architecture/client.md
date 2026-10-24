# client

> **正本**: [`client/`](../../../client/)（Cargo workspace）  
> **ステータス**: Slice B（Subscribe + InputEvent Publish + 最小 wgpu）

## レイアウト

| パス | 役割 |
|:---|:---|
| `client/Cargo.toml` | workspace 根（将来 submodule の根） |
| `client/app` | 画面プロセス（パッケージ名 `client`） |
| `client/host-server` | 裏プロセスの計算ホスト |
| `../protocol` | 共有ワイヤ契約（workspace 外） |

## 保証すること / しないこと

| 保証する | 保証しない |
|:---|:---|
| Zenoh client として frame Subscribe / input Publish | ルーター機能（他人の通信の中継は禁止） |
| world-server から Room Token を取得して InputEvent に添付 | auth ログイン UI（Slice B は匿名 Token 可） |
| RenderFrame decode → 画面上の移動表示 | OSC / Dual Editor / アセット LOD |
| | launcher 本実装 |

## 関連

- ワイヤ契約: [protocol.md](protocol.md)
- 転送: [zenoh-router.md](zenoh-router.md)
- Room Token: [world-server.md](world-server.md)
