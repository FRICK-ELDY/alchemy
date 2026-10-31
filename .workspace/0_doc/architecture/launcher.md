# Launcher

> **正本**: [`launcher/`](../../../launcher/)（`alchemy-launcher`）  
> **ステータス**: 最小実装 done（x0032 / P4-4）。再起動ポリシー単体 + CI（x0047 / P3-2）

## 保証すること / しないこと

| 保証する | 保証しない |
|:---|:---|
| host-server と client を子プロセスとして起動 | `alchemy://` ディープリンク |
| **Exit Client only** → client 終了後も host 維持 | トレイ UI / インストーラ |
| **Exit Client+Host** → IPC で host を止め、host exit 0 なら **launcher も終了**（再起動しない） | Zenoh `host_control` 経由の停止（launcher 経路では使わない） |
| host が `127.0.0.1:0` に bind し、ready ファイルの `ipc=` で通知（ポート TOCTOU なし） | Compose への組み込み |
| host **非 0** crash 時のみ backoff 再起動（上限 3・`decide_host_exit`）。launcher 側 `ipc_addr` は更新 | host handoff / Zenoh liveliness 監視；**respawn 後 ready 待ち失敗**の予算内再試行（即 shutdown） |
| Ctrl+C で IPC shutdown → 子終了（再起動後も launcher 保持の新 addr を使用） | **host crash 再起動後の client 側 Exit Client+Host**（client env の旧 `ALCHEMY_HOST_IPC_ADDR` のまま）・`cargo run` 経由時の `kill` で cargo 孫プロセスが orphan になり得ること（`ALCHEMY_HOST_BIN` 推奨） |
| exit 0 を意図的停止とみなす（現状 host は IPC → main leave → `Ok(())` のみ） | 将来の「exit 0 だが再起動したい」終了理由の区別 |
| | multi-room / 複数 host |

## 終了の境界

| 操作 | host | launcher |
|:---|:---|:---|
| Exit Client only | 維持 | 監視継続 |
| Exit Client+Host（IPC SHUTDOWN） | exit 0（意図的） | **終了**（再起動しない） |
| host crash（非 0） | — | backoff 再起動（launcher の IPC addr 更新；client UI の Exit Host は非保証） |
| Ctrl+C | IPC で停止 | 終了 |

## 環境変数

| 変数 | 役割 |
|:---|:---|
| `ALCHEMY_CLIENT_DIR` | client Cargo workspace 根（既定: repo `client/`） |
| `ALCHEMY_HOST_BIN` / `ALCHEMY_CLIENT_BIN` | 直接バイナリ（未設定時は `cargo run -p …`） |
| （未設定時）`connect.json` | 起動前に `alchemy-connect-config` が端点を env へ注入（CI env が勝ち・x0066） |
| `ALCHEMY_HOST_IPC_TOKEN` | launcher → host。host が `127.0.0.1:0` で listen |
| `ALCHEMY_HOST_IPC_ADDR` | launcher が ready の `ipc=` から client に渡す（loopback のみ） |
| `ALCHEMY_HOST_READY_FILE` | `ready\nipc=127.0.0.1:port\n`（最初の frame put 後） |

偽 Zenoh `host_control` との関係: [trust-boundary.md](trust-boundary.md)。

## 起動

```bash
# auth / world / assets / zenohd を日常スタックで起動したうえで:
cd launcher
cargo run -p alchemy-launcher
```

Windows / Unix 共通。詳細手順の日常正本は [README Desktop 手通し](../../../README.md)（手動 `cargo run`）と併用可。

## 配布 ZIP（Windows x64）

テスター向けは `ALCHEMY_HOST_BIN` / `ALCHEMY_CLIENT_BIN` を同梱 exe に向けた ZIP。  
手順: [windows-tester-zip.md](../manual/windows-tester-zip.md)。ローカル煙は `pack-windows-zip.ps1`、タグ push は `publish-windows-release.ps1`。Pre-release 作成は CI（`windows-release.yml`）。  
`Start.cmd` が bin パスをセットして `alchemy-launcher.exe` を起動する。
