# x0072 — launcher discovery（host IPC 再通知）

> **ID:** `x0072` · **slug:** `launcher-discovery` · **status:** `done` · **area:** `[launcher, client, protocol, docs]` · **depends_on:** `[x0032, x0047]`  
> **優先度:** backlog **P1-5**（improvement-plan 2026-10-30 **P1-4**）

目標: launcher プロセス上のセッション固定 discovery endpoint で、host crash 再起動後も client の Exit Client+Host が新しい host IPC addr に届くようにする。

根拠: [backlog-priority-2026-11-04.md](../backlog-priority-2026-11-04.md) **P1-5** · [improvement-plan-2026-10-30.md](../../0_doc/evaluation/improvement-plan-2026-10-30.md) **P1-4** · [launcher.md](../../0_doc/architecture/launcher.md)

## 保証する / しない

| 保証する | 保証しない |
|:---|:---|
| host crash 再起動後も client Exit Client+Host が新 IPC に届く | deep link / tray / handoff |
| discovery は loopback + token、generation 単調増加 | well-known 固定ポート・複数 launcher 同居の衝突回避 |
| discovery 無し起動は従来 env IPC のまま | 常時バックグラウンド更新・Zenoh 経由の addr 配布 |

## 設計（要点）

- launcher が `127.0.0.1:0` で discovery listen（プロセス寿命＝セッション固定）
- client へ `ALCHEMY_LAUNCHER_DISCOVERY_ADDR` + 既存 `ALCHEMY_HOST_IPC_TOKEN`
- 行プロトコル: `LOOKUP <token>` → `OK gen=<u64> ipc=127.0.0.1:<port>` / `ERR`
- client は shutdown 直前に LOOKUP → 得た addr で既存 `SHUTDOWN <token>`
- host ready / restart のたびに discovery state を `publish`（gen++）
- host restart **開始時**に `invalidate`（LOOKUP→`ERR`）し、死んだポートへの偽 OK を避ける
- discovery env があるとき client は LOOKUP 失敗で `ALCHEMY_HOST_IPC_ADDR` にフォールバックしない

## 完了条件

1. protocol / launcher discovery 単体テスト緑（LOOKUP auth・gen 増加・parse）
2. client `try_launcher_ipc` が discovery → shutdown の順
3. `cargo test -p alchemy-protocol` / `cargo test -p alchemy-launcher` 緑、clippy `-D warnings`
4. Slice・architecture・roadmap / backlog 更新

## Phase

| Phase | 内容 | 状態 |
|:---|:---|:---|
| 0 | Slice + roadmap / backlog / architecture | done |
| 1 | protocol LOOKUP helpers | done |
| 2 | launcher discovery listen + publish | done |
| 3 | client resolve-on-shutdown | done |
| 4 | test / clippy | done |

← [目次](../README.md)
