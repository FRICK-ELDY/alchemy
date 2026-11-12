# x009c — bind 全体上限 + session_pub 起点（P2-14）

> **ID:** `x009c` · **slug:** `bind-global-session-pub` · **status:** `done` · **area:** `[host, client, docs]` · **depends_on:** `[x0040, x0077]`  
> **優先度:** improvement **P2-14**

目標: bind レートの起点を自己申告 ZenohId から **Ed25519 検証済み `session_pub`** に移し、host 全体 soft-global（成功 JWT で返却）で JWT CPU 天井を戻す。

根拠: [improvement-plan-2026-11-05.md](../../0_doc/evaluation/improvement-plan-2026-11-05.md) **P2-14** · [opus-specific-weaknesses-2026-11-05.md](../../0_doc/evaluation/opus/opus-specific-weaknesses-2026-11-05.md)

---

## 保証する / しない

| 保証する | 保証しない |
|:---|:---|
| bind キーは Ed25519 後の `session_pub`（Zenoh Sample ID に依存しない） | 初回 `session_pub` 出現への PoW |
| Ed25519 後・JWT 前に soft-global（20 / 10/s）+ per-pub（5 / 2/s）を原子消費（Leave 非免除） | 失敗 JWT 洪水で global が空のときの正規入室遅延ゼロ |
| 成功 JWT は soft-global を 1 トークン返却（正規 join が他者を恒久枯渇させない） | temporary ban / strike / Zenoh ACL |
| 失敗 JWT は global 消費を維持；per-pub の `forget` は **SessionTable 未登録時のみ**（既 bind rebind 失敗では per-pub を残す） | 未 bind steady の Ed25519 洪水 |
| `by_session_pub.len() ≤ MAX_BIND_PUBS`；idle は **成功消費**基準（deny/refill は延ばさない） | 満杯かつ全行 hot のときの新規 pub deny |
| deny で map 増殖しない（消費成功時のみ insert） | mTLS 証明書単位の分割 |

---

## 設計（要点）

```
Ed25519 ok
    │
    ▼
allow_bind(session_pub): soft-global ∩ per-pub 原子消費
    │
    ├─ JWT fail  → global は返さない；sessions→rates をネスト保持した臨界区域で
    │              is_bound → after_bind_jwt_fail（未登録なら forget）
    │
    └─ JWT ok    → refund_bind_global + commit_bind
```

- x0077 の per-ZenohId `by_bind_source` は撤去（自己申告 ID で 256×予算が広がる経路を閉じる）
- client の `session.new_timestamp()` は Zenoh causality 用に残す（bind キーではない）

---

## Phase

| Phase | 内容 | 状態 |
|:---|:---|:---|
| 0 | Slice + roadmap / improvement | **done** |
| 1 | `intent_rate` session_pub + global refund | **done** |
| 2 | host gate + docs + tests | **done** |

---

## 完了条件

1. 同一 `session_pub` の連打は per-pub で止まり、Sample ID を変えても予算がリセットされない
2. 成功 refund 後は別 pub が soft-global で通る；失敗 JWT は global を返さない
3. pub map は cap 超過せず、idle GC 後に新規 pub を受理する
4. `cargo test -p host-server -- intent_rate` 緑
5. improvement P2-14 を done 表記

## やらない（本 Slice）／次

- 初回 `session_pub` への PoW
- Intent temporary ban / strike
- mTLS CN 単位のレート分割

## 関連

- [x0040-intent-rate-jwks-hard-stale.md](x0040-intent-rate-jwks-hard-stale.md)
- [x0077-bind-source-split.md](x0077-bind-source-split.md)
- [improvement-plan-2026-11-05.md](../../0_doc/evaluation/improvement-plan-2026-11-05.md)
- `client/host-server/src/intent_rate.rs` · `client/host-server/src/main.rs`

← [目次](../README.md)
