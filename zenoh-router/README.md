# zenoh-router（ローカル）

Slice A/B の入り口。ローカル `zenohd` が空間同期の pub/sub を仲介する。  
Room Token の検証は zenohd ではなく host-server が行う。

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
| InputEvent | `alchemy/dev/room/{room_id}/input` |

例: `alchemy/dev/room/local/frame` / `alchemy/dev/room/local/input`

将来は `alchemy/tenant/{運営者ID}/room/{room_id}/**` へ拡張する。

## 起動

前提: `zenohd` が PATH にあること（例: `cargo install zenohd`）。

```bash
# macOS / Linux / WSL / Git Bash
zenohd -c zenoh-router/config.json5
# または
cd zenoh-router && zenohd -c config.json5
```

```powershell
# Windows PowerShell
cd zenoh-router
zenohd -c config.json5
```

既定リスナ: `tcp/127.0.0.1:7447` と **`tcp/0.0.0.0:7447`**（x0014 LAN）。

host-server / client は **client モード**で接続する。エンドポイントは環境変数 **`ZENOH_CONNECT`**（既定 `tcp/127.0.0.1:7447`）。LAN 上の他マシンからは `ZENOH_CONNECT=tcp/<PC_IP>:7447`。

## トークン検証

Slice A では未実装。接続時の JWT / Room Token 検証は後続（world-server / Relay Agent）。
