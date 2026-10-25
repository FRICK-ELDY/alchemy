# zenoh-router

> **正本**: [`zenoh-router/`](../../../zenoh-router/)  
> **ステータス**: Slice A done / Slice B でもローカル zenohd のみ

## 保証すること / しないこと

| 保証する | 保証しない |
|:---|:---|
| 同一マシン / LAN の pub-sub | JWKS / Room Token ACL（検証は host-server） |
| TCP リスナ `127.0.0.1:7447` + `0.0.0.0:7447`（x0014） | CCU・帯域 QoS |
| client/host の `ZENOH_CONNECT`（既定 localhost） | VPS ブリッジ・マルチテナント隔離 |

## MVP キー式

| 用途 | キー |
|:---|:---|
| RenderFrame | `alchemy/dev/room/{room_id}/frame` |
| InputEvent | `alchemy/dev/room/{room_id}/input` |

既定 `room_id`: `local`

詳細・起動手順は [zenoh-router/README.md](../../../zenoh-router/README.md)。

## トークン検証

Slice B では **zenohd は Token を見ない**。host-server が入力ペイロード内の Room Token を JWKS で検証する。
