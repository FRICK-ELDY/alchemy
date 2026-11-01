# relay-server

> **正本**: [`relay-server/`](../../../relay-server/)  
> **ステータス**: ローカル done · WAN TCP（x001b）→ **WAN TLS（x0022）** → **WAN mTLS + role ACL（x004e）**  
> **役割**: zenohd の設定・待受（空間同期の仲介）。Allocation → mTLS は **Relay Agent**（[`relay-agent/`](../../../relay-agent/)）

## 保証すること / しないこと

| 保証する | 保証しない |
|:---|:---|
| 同一マシン / LAN の pub-sub（`config.json5` tcp・ACL オフ） | JWKS / Room Token ACL（検証は host-server） |
| TCP リスナ `0.0.0.0:7447`（localhost + LAN、x0014） | CCU・帯域 QoS；**router max payload**（アプリ側 decode 上限は [protocol.md](protocol.md) x003b） |
| WAN: ConoHa 上 zenohd の **TLS+mTLS** 待受（x004e）+ IP allowlist | Billing / 第三者リレー / ACL ホットリロード |
| WAN: role CN ACL（`alchemy-host` / `alchemy-guest`）+ default deny | per-room 動的 subject（部屋隔離は `wire_cap`） |
| client/host の `ZENOH_CONNECT`（既定 localhost `tcp`；WAN は `tls/` + allocate） | Relay Allocation Token の課金フィールド |

## 設定ファイル

| ファイル | 用途 |
|:---|:---|
| [`config.json5`](../../../relay-server/config.json5) | ローカル / LAN（multicast ON・tcp・ACL オフ） |
| [`config.wan.json5`](../../../relay-server/config.wan.json5) | ConoHa 公網（multicast OFF・**tls+mTLS+ACL**） |

運用手順（WAN 正本: mTLS / Relay Agent / allowlist）: [`manual/relay-server-wan.md`](../manual/relay-server-wan.md)  
DNS / Let's Encrypt（追加）: [`manual/relay-server-dns.md`](../manual/relay-server-dns.md)

## MVP キー式（x004e）

| 用途 | キー |
|:---|:---|
| RenderFrame | `alchemy/dev/room/{room_id}/{wire_cap}/frame` |
| InputEvent | `alchemy/dev/room/{room_id}/{wire_cap}/input` |
| SceneManifest | `alchemy/dev/room/{room_id}/{wire_cap}/manifest` |
| PresenceState | `alchemy/dev/room/{room_id}/{wire_cap}/presence` |
| Snapshot | `alchemy/dev/room/{room_id}/{wire_cap}/snapshot` |
| Liveliness | `alchemy/dev/room/{room_id}/{wire_cap}/alive/{sub}/{jti}` |
| Host control | `alchemy/dev/room/{room_id}/{wire_cap}/host_control`（**既定オフ**・x001e） |

`wire_cap` は world が room 作成時に発行し、Room Token / Allocation Token 応答で配布する。

偽 publish 時の防御責任: [trust-boundary.md](trust-boundary.md)。

## トークン検証

zenohd は Token を見ない。**Relay Agent** が Allocation Token（`aud=alchemy-relay`）を JWKS で検証してから短命 mTLS 証明書を渡す。Room Token は従来どおり host-server。アプリ層の境界は [trust-boundary.md](trust-boundary.md)。
