# x00d3 — relay-agent JWKS を ureq 化（P2-7）

> **ID:** `x00d3` · **slug:** `relay-agent-ureq-jwks` · **status:** `done` · **area:** `[relay, docs]` · **depends_on:** `[x0083, x00d2]`  
> **優先度:** improvement-plan **P2-7**（負債・衛生）

目標: JWKS 取得を `curl` 子プロセスから **ureq** に置き換え、JWKS cache lock の `expect` を解消する。依存追加は 2026-11-17 人承認済み。

根拠: [improvement-plan-2026-11-12.md](../../0_doc/evaluation/improvement-plan-2026-11-12.md) **P2-7** · [opus-specific-weaknesses-2026-11-12.md](../../0_doc/evaluation/opus/opus-specific-weaknesses-2026-11-12.md)

---

## 保証する / しない

| 保証する | 保証しない |
|:---|:---|
| JWKS GET は ureq（タイムアウト付き）で、PATH 上の curl に依存しない | openssl 子プロセスの除去（CSR 署名は継続） |
| JWKS `RwLock` の poison で panic しない（本番経路・`jwks_age` / test helper） | 分散 JWKS キャッシュ |

---

## Phase

| Phase | 内容 | 状態 |
|:---|:---|:---|
| 0 | Slice + 目次 | **done** |
| 1 | `ureq` 依存 + `ureq_get_json` 実装 + `expect` 除去 | **done** |
| 2 | `cargo test` / docs / improvement-plan | **done** |

---

## 完了条件

1. `Command::new("curl")` が JWKS 経路に残っていない
2. `expect("jwks lock")` が残っていない
3. `cargo test`（relay-agent）Green
4. improvement-plan P2-7 closed

## やらない

- P2-4 / P2-8 / ~~PoP nonce~~ → [x00d4](x00d4-pop-nonce.md)
- openssl 依存の除去

← [目次](../README.md) · [x00d2](x00d2-catalog-rl-epoch-api.md) · [improvement-plan](../../0_doc/evaluation/improvement-plan-2026-11-12.md)
