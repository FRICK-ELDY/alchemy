# client

> **正本**: [`crates/client/`](../../../crates/client/)  
> **ステータス**: Slice A（Subscribe + 最小 wgpu）

## 保証すること / しないこと

| 保証する | 保証しない |
|:---|:---|
| Zenoh client として `alchemy/dev/room/local/frame` を Subscribe | ルーター機能（他人の通信中継は禁止） |
| RenderFrame decode → 画面上の移動表示 | OSC / Dual Editor / アセット LOD |
| | auth ログイン UI / Room Token |
| | host-server とのローカル IPC |

## 関連

- 計算権威: [`crates/host-server/`](../../../crates/host-server/)（別プロセス）
- ワイヤ契約: [protocol.md](protocol.md)
- 転送: [zenoh-router.md](zenoh-router.md)
