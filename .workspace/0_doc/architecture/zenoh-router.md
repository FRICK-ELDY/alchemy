# zenoh-router

> **正本**: [`zenoh-router/`](../../../zenoh-router/)  
> **ステータス**: ローカル done · WAN（x001b）done

## 保証すること / しないこと

| 保証する | 保証しない |
|:---|:---|
| 同一マシン / LAN の pub-sub | JWKS / Room Token ACL（検証は host-server） |
| TCP リスナ `0.0.0.0:7447`（localhost + LAN、x0014） | CCU・帯域 QoS |
| WAN: ConoHa 上 zenohd + IP allowlist（x001b） | 拠点 zenohd ↔ VPS ブリッジ・マルチテナント隔離 |
| client/host の `ZENOH_CONNECT`（既定 localhost） | Relay Allocation Token / QoS |

## 設定ファイル

| ファイル | 用途 |
|:---|:---|
| [`config.json5`](../../../zenoh-router/config.json5) | ローカル / LAN（multicast ON） |
| [`config.wan.json5`](../../../zenoh-router/config.wan.json5) | ConoHa 公網（multicast OFF） |

運用手順（Ubuntu 22.04 / systemd / ufw）: [`manual/zenoh-wan-conoha.md`](../manual/zenoh-wan-conoha.md)

## MVP キー式

| 用途 | キー |
|:---|:---|
| RenderFrame | `alchemy/dev/room/{room_id}/frame` |
| InputEvent | `alchemy/dev/room/{room_id}/input` |

既定 `room_id`: `local`

詳細・起動手順は [zenoh-router/README.md](../../../zenoh-router/README.md)。

## トークン検証

zenohd は Token を見ない。host-server が入力ペイロード内の Room Token を JWKS で検証する。
