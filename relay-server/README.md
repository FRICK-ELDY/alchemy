# relay-server

ローカル / WAN（ConoHa）の `zenohd` 設定・待受。空間同期の pub/sub を仲介する。  
Room Token の検証は host-server。Allocation Token → mTLS は [relay-agent](../relay-agent/)（x004e）。

| | 役割 |
|:---|:---|
| **relay-server** | zenohd の設定・待受（空間同期の仲介） |
| **relay-agent** | Allocation Token 検証 → 短命 mTLS 証明書発行 |

正本アーキテクチャ: [`.workspace/0_doc/architecture/relay-server.md`](../.workspace/0_doc/architecture/relay-server.md)  
WAN 運用正本: [`.workspace/0_doc/manual/relay-server-wan.md`](../.workspace/0_doc/manual/relay-server-wan.md)（x001b → TLS x0022 → **mTLS x004e**）  
DNS / Let's Encrypt（追加）: [`.workspace/0_doc/manual/relay-server-dns.md`](../.workspace/0_doc/manual/relay-server-dns.md)

## 保証すること / しないこと

| 保証する | 保証しない |
|:---|:---|
| 同一マシン / LAN /（WAN 時）allowlist 経由のプロセスが同じ zenohd に接続できる | JWKS を zenohd が直接見る |
| WAN 待受は TLS+mTLS（平文 TCP のみの公網公開をしない） | Billing / CCU QoS |
| WAN: role CN ACL（host/guest）+ default deny + ingress/egress on `room/**`；SHM off | per-room 動的 ACL（部屋は `wire_cap`）；キー族 put 分離（暫定緩和） |
| キー式での Publish / Subscribe | 拠点 zenohd ↔ VPS ブリッジ |

## ローカル vs WAN

| | ローカル [`config.json5`](config.json5) | WAN [`config.wan.json5`](config.wan.json5) |
|:---|:---|:---|
| 用途 | 開発・LAN（x0014） | ConoHa 公網（x004e mTLS） |
| listen | `tcp/0.0.0.0:7447` | `tls/0.0.0.0:7447` |
| mTLS / ACL | オフ | **ON** |
| multicast scouting | **ON** | **OFF** |
| 公開面 | LAN / ファイアウォール任意 | **TLS+mTLS + ufw allowlist** |

## MVP キー式（x004e）

| 用途 | キー式 |
|:---|:---|
| RenderFrame | `alchemy/dev/room/{room_id}/{wire_cap}/frame` |
| InputEvent | `alchemy/dev/room/{room_id}/{wire_cap}/input` |

将来は `alchemy/tenant/{運営者ID}/room/{room_id}/**` へ拡張する。

## 起動（ローカル）

前提: `zenohd` が PATH にあること（例: `cargo install zenohd`）。

```bash
zenohd -c relay-server/config.json5
```

証明書生成（WAN）: `bash relay-server/scripts/gen-wan-dev-certs.sh <host> ./tls-wan-dev`
