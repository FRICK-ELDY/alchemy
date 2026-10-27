# x0032 — launcher 最小実装（P4-4）

> **ID:** `x0032` · **slug:** `launcher-minimal` · **status:** `done` · **area:** `[launcher, host, client, docs]` · **depends_on:** `[x001e, x0031]`

目標: `launcher/` を最小プロセスマネージャとして実装する。client 終了 ≠ host 終了。shutdown は localhost IPC。

正本: [launcher.md](../../0_doc/architecture/launcher.md) · [trust-boundary.md](../../0_doc/architecture/trust-boundary.md)

## 設計

| 層 | 内容 |
|:---|:---|
| launcher | spawn host → ready（`ipc=`）待ち → spawn client；exit 0=意図的停止で終了；非 0 のみ backoff |
| host | token 時に `127.0.0.1:0` bind；SHUTDOWN で main loop leave（exit 0） |
| client | Exit Client+Host は IPC 優先（loopback 検証）；単独起動経路は維持 |

## やらない

- deep link / トレイ / handoff / インストーラ
- Compose 組み込み
- Zenoh host_control の削除（opt-in stub は残す）

## 成功条件

1. `cargo run`（launcher）で host+client が立ち上がる手順が `launcher.md` にある
2. client を閉じても host が残る設計になっている
3. launcher 終了で host が IPC 経由で止まる
4. host IPC 単体テスト緑

← [目次](../README.md)
