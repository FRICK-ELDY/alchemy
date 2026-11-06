# Launcher（廃止）

> **ステータス**: **廃止** — 責務は [`client` の `HostSupervisor`](client.md) に移譲  
> **旧正本**: 削除済み `launcher/`（`alchemy-launcher`）

## プロダクト決断

**Exit Client only（画面を閉じてもローカル host 維持）は破棄。**  
部屋継続は Handover（MVP stub → 失敗時は部屋終了）または運営 headless に一本化する。

## 移譲表

| 旧 launcher 機能 | 現行 |
|:---|:---|
| 常時 host+client 同時起動 | **廃止** — Steam/起動は **client のみ** |
| Exit Client only | **廃止** |
| discovery LOOKUP / gen++（x0072） | **廃止** — 親 client が IPC addr を保持 |
| host crash backoff 再起動 | **廃止** — ローカルは fail→Handover stub/部屋終了。常時部屋は cloud headless |
| localhost IPC SHUTDOWN + token | **client `HostSupervisor`** + `protocol` `host_ipc` |
| ready ファイル `ipc=` | **維持**（host が `127.0.0.1:0` bind） |
| 道連れ終了 | Windows Job Object / Linux `PR_SET_PDEATHSIG` |
| 配布 `Start.cmd` → launcher | **client.exe 直起動**（`ALCHEMY_HOST_BIN` で同梱 host） |

## 起動（現行）

```bash
# auth / world / assets / zenohd を日常スタックで起動したうえで:
cd client
ALCHEMY_FORCE_DESKTOP=1 cargo run -p client
# 「Start session」で初めて host-server が子プロセス起動。Join では起動しない。
```

Windows テスター ZIP: [windows-tester-zip.md](../manual/windows-tester-zip.md)。

## 偽 Zenoh `host_control` との関係

[trust-boundary.md](trust-boundary.md)。supervisor 経路は localhost IPC。Zenoh `host_control` は opt-in のまま。
