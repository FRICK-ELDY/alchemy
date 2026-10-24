# zenoh-router

> **正本**: [`zenoh-router/`](../../../zenoh-router/)  
> **ステータス**: Slice A（ローカル zenohd のみ）

## 保証すること / しないこと

| 保証する | 保証しない |
|:---|:---|
| 同一マシン / LAN の pub-sub | JWKS / Room Token ACL |
| TCP リスナ `127.0.0.1:7447` | CCU・帯域 QoS |
| | VPS ブリッジ・マルチテナント隔離 |

## MVP キー式

`alchemy/dev/room/{room_id}/frame`  
既定: `alchemy/dev/room/local/frame`

詳細・起動手順は [zenoh-router/README.md](../../../zenoh-router/README.md)。

## トークン検証

Slice A では未実装。
