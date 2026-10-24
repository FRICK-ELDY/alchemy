# zenoh-router（ローカル）

Slice A の入り口。ローカル `zenohd` が空間同期の pub/sub を仲介する。

## 保証すること / しないこと

| 保証する | 保証しない |
|:---|:---|
| 同一マシン（または LAN）のプロセスが同じ zenohd に接続できる | JWKS / Room Token による ACL |
| キー式での Publish / Subscribe | CCU・帯域 QoS |
| | ConoHa VPS ブリッジ・サードパーティリレー |
| | マルチテナント名前空間隔離 |

## MVP キー式

| 用途 | キー式 |
|:---|:---|
| RenderFrame | `alchemy/dev/room/{room_id}/frame` |

例: `alchemy/dev/room/local/frame`（host-server / client の既定）

将来は `alchemy/tenant/{運営者ID}/room/{room_id}/**` へ拡張する。

## 起動（Windows）

前提: `zenohd` が PATH にあること（例: `cargo install zenohd`）。

```powershell
cd D:\github\frick\alchemy\zenoh-router
zenohd -c config.json5
```

既定リスナ: `tcp/127.0.0.1:7447`

host-server / client は **client モード**でこのエンドポイントへ接続する。

## トークン検証

Slice A では未実装。接続時の JWT / Room Token 検証は後続（world-server / Relay Agent）。
