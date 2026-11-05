# Launcher

> **正本**: [`launcher/`](../../../launcher/)（`alchemy-launcher`）  
> **ステータス**: 最小実装 done（x0032 / P4-4）。再起動ポリシー単体 + CI（x0047 / P3-2）。discovery（x0072 / P1-5）

## 保証すること / しないこと

| 保証する | 保証しない |
|:---|:---|
| host-server と client を子プロセスとして起動 | `alchemy://` ディープリンク |
| **Exit Client only** → client 終了後も host 維持 | トレイ UI / インストーラ |
| **Exit Client+Host** → IPC で host を止め、host exit 0 なら **launcher も終了**（再起動しない） | Zenoh `host_control` 経由の停止（launcher 経路では使わない） |
| host が `127.0.0.1:0` に bind し、ready ファイルの `ipc=` で通知（ポート TOCTOU なし） | Compose への組み込み |
| host **非 0** crash 時のみ backoff 再起動（上限 3・`decide_host_exit`）。launcher 側 `ipc_addr` は更新 | host handoff / Zenoh liveliness 監視；**respawn 後 ready 待ち失敗**の予算内再試行（即 shutdown） |
| Ctrl+C で IPC shutdown → 子終了（再起動後も launcher 保持の新 addr を使用） | well-known 固定ポート・複数 launcher 同居の衝突回避；`cargo run` 経由時の `kill` で cargo 孫プロセスが orphan になり得ること（`ALCHEMY_HOST_BIN` 推奨） |
| exit 0 を意図的停止とみなす（現状 host は IPC → main leave → `Ok(())` のみ） | 将来の「exit 0 だが再起動したい」終了理由の区別 |
| **discovery**（`127.0.0.1:0`・セッション固定）で host generation / IPC addr を client へ再通知。Exit Client+Host は LOOKUP → SHUTDOWN | 常時バックグラウンド更新・Zenoh 経由の addr 配布；discovery 無し手動起動の stale env（従来どおり） |
| | multi-room / 複数 host |

## 終了の境界

| 操作 | host | launcher |
|:---|:---|:---|
| Exit Client only | 維持 | 監視継続 |
| Exit Client+Host（discovery LOOKUP → IPC SHUTDOWN） | exit 0（意図的） | **終了**（再起動しない） |
| host crash（非 0） | — | discovery **invalidate**（LOOKUP→ERR）→ backoff 再起動 → publish（gen++ / 新 ipc）；client は LOOKUP 成功後にのみ SHUTDOWN |
| Ctrl+C | IPC で停止 | 終了 |

## 環境変数

| 変数 | 役割 |
|:---|:---|
| `ALCHEMY_CLIENT_DIR` | client Cargo workspace 根（既定: repo `client/`） |
| `ALCHEMY_HOST_BIN` / `ALCHEMY_CLIENT_BIN` | 直接バイナリ（未設定時は `cargo run -p …`） |
| `ALCHEMY_HOST_IPC_TOKEN` | launcher → host / client。host が `127.0.0.1:0` で listen；discovery LOOKUP も同一 token |
| `ALCHEMY_HOST_IPC_ADDR` | launcher が ready の `ipc=` から client に渡す（loopback のみ・初回互換）。再起動後は discovery が正 |
| `ALCHEMY_LAUNCHER_DISCOVERY_ADDR` | launcher discovery（`127.0.0.1:port`）。client は shutdown 直前に `LOOKUP <token>` |
| `ALCHEMY_HOST_READY_FILE` | `ready\nipc=127.0.0.1:port\n`（最初の frame put 後） |

偽 Zenoh `host_control` との関係: [trust-boundary.md](trust-boundary.md)。

## Discovery プロトコル（x0072）

1 行テキスト（既存 host IPC と同流儀）。

| 方向 | 行 |
|:---|:---|
| client → launcher | `LOOKUP <token>\n` |
| launcher → client | `OK gen=<u64> ipc=127.0.0.1:<port>\n` |
| 失敗 | `ERR\n`（bad token / まだ ready なし） |

正本 helpers: [`protocol/src/host_ipc.rs`](../../../protocol/src/host_ipc.rs)。

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
