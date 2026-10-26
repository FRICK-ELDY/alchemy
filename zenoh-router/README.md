# zenoh-router

ローカル / WAN（ConoHa）の `zenohd` 設定。空間同期の pub/sub を仲介する。  
Room Token の検証は zenohd ではなく host-server が行う。

正本アーキテクチャ: [`.workspace/0_doc/architecture/zenoh-router.md`](../.workspace/0_doc/architecture/zenoh-router.md)  
WAN 運用手順: [`.workspace/0_doc/manual/zenoh-wan-conoha.md`](../.workspace/0_doc/manual/zenoh-wan-conoha.md)（x001b）

## 保証すること / しないこと

| 保証する | 保証しない |
|:---|:---|
| 同一マシン / LAN /（WAN 時）allowlist 経由のプロセスが同じ zenohd に接続できる | JWKS / Room Token による ACL |
| キー式での Publish / Subscribe | CCU・帯域 QoS |
| | 拠点 zenohd ↔ VPS ブリッジ・マルチテナント隔離 |

## ローカル vs WAN

| | ローカル [`config.json5`](config.json5) | WAN [`config.wan.json5`](config.wan.json5) |
|:---|:---|:---|
| 用途 | 開発・LAN（x0014） | ConoHa 公網待受（x001b） |
| listen | `tcp/0.0.0.0:7447` | 同じ |
| multicast scouting | **ON** | **OFF** |
| 公開面 | LAN / ファイアウォール任意 | **ufw IP allowlist 必須**（手順書） |

## MVP キー式

| 用途 | キー式 |
|:---|:---|
| RenderFrame | `alchemy/dev/room/{room_id}/frame` |
| InputEvent | `alchemy/dev/room/{room_id}/input` |

例: `alchemy/dev/room/local/frame` / `alchemy/dev/room/local/input`

将来は `alchemy/tenant/{運営者ID}/room/{room_id}/**` へ拡張する。

## 起動（ローカル）

前提: `zenohd` が PATH にあること（例: `cargo install zenohd`）。

```bash
# macOS / Linux / WSL / Git Bash
zenohd -c zenoh-router/config.json5
```

```powershell
# Windows PowerShell
cd zenoh-router
zenohd -c config.json5
```

既定リスナ: **`tcp/0.0.0.0:7447`**（localhost + LAN）。`127.0.0.1` と併記すると Linux で Address already in use になる。

host-server / client は **client モード**で接続する。エンドポイントは環境変数 **`ZENOH_CONNECT`**（既定 `tcp/127.0.0.1:7447`）。LAN 上の他マシンからは `ZENOH_CONNECT=tcp/<PC_IP>:7447`。

## 起動（WAN / VPS）

VPS 上では [`config.wan.json5`](config.wan.json5) を使う。systemd・ufw・クライアント env は手順書へ:

[`.workspace/0_doc/manual/zenoh-wan-conoha.md`](../.workspace/0_doc/manual/zenoh-wan-conoha.md)

```bash
# VPS 上の例
zenohd -c /etc/alchemy/zenoh-wan.json5
# クライアント側
export ZENOH_CONNECT=tcp/<VPS_IP>:7447
```

## トークン検証

zenohd は Token を見ない。接続時の JWT / Room Token 検証は host-server（および後続の Relay Agent）。
