# x0047 — launcher CI + 再起動ポリシー単体テスト

> **ID:** `x0047` · **slug:** `launcher-ci-restart-policy` · **status:** `done` · **area:** `[launcher, ci, docs]` · **depends_on:** `[x0032]`  
> **優先度:** **P3-2**

目標: `alchemy-launcher` をローカル／GitHub CI のゲートに載せ、host crash 再起動ポリシー（意図的 exit 0・backoff・予算）を純粋関数の単体テストで固定する。clippy `-D warnings` 緑を維持する。

根拠: [improvement-plan-2026-10-28.md](../../0_doc/evaluation/improvement-plan-2026-10-28.md) **P3-2**

正本: [launcher.md](../../0_doc/architecture/launcher.md)

## 設計

| 層 | 内容 |
|:---|:---|
| policy | [`launcher/src/restart.rs`](../../../launcher/src/restart.rs) — `decide_host_exit` / `backoff_for_attempt` |
| GitHub | `.github/workflows/rust-ci.yml` `launcher-check`（x003c で追加済み・本帯で明示） |
| ローカル | `make ci-launcher` / `.scripts/ci.sh launcher`（`make ci` の `all` にも含む） |

## 保証する / しない

| 保証する | 保証しない |
|:---|:---|
| exit 0 → launcher 終了（再起動しない）の単体固定 | deep link / tray / handoff（→ P4-2） |
| 非 0 crash は最大 3 回・1s/2s/4s backoff で再起動（`decide_host_exit`） | client の旧 IPC addr 更新 |
| fmt / clippy `-D warnings` / build / test が CI・`make ci-launcher` で走る | Compose への launcher 組み込み |
| | **respawn 後の ready 待ち失敗**を予算内で再試行すること（即 shutdown で確定） |

## 完了条件

- `cargo test`（launcher）で再起動ポリシー緑
- `cargo clippy -- -D warnings` 緑
- `make ci-launcher`（または同等）緑

← [目次](../README.md)
