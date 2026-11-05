# x0077 — bind 送信元分割（P2-5）

> **ID:** `x0077` · **slug:** `bind-source-split` · **status:** `done` · **area:** `[host, client, docs]` · **depends_on:** `[x0040, x0076]`

目標: improvement-plan **P2-5** — bind の host 全体バケットを **Zenoh timestamp の ZenohId 単位**に分割し、未知鍵洪水が他ピアの入室を止めないようにする。

根拠: [backlog-priority-2026-11-04.md](../backlog-priority-2026-11-04.md) **P3** · [improvement-plan-2026-10-30.md](../../0_doc/evaluation/improvement-plan-2026-10-30.md) **P2-5** · [opus-specific-weaknesses-2026-10-30.md](../../0_doc/evaluation/opus/opus-specific-weaknesses-2026-10-30.md)

## 保証する / しない

| 保証する | 保証しない |
|:---|:---|
| ある ZenohId の bind 消費が、別 ZenohId の bind 予算を減らさない | timestamp ID 偽造ローテによる JWT CPU 上限 |
| Ed25519 後・JWT 前に per-source + per-`session_pub` を原子消費（Leave 非免除） | mTLS 証明書単位の分割（Sample から不可） |
| 消費成功時のみ source / pub 行を insert（deny で map 増殖しない） | Zenoh `unstable` SourceInfo |
| timestamp 無しは共有 `unknown` キー（旧 host-global 相当） | unknown 同士の相互干渉ゼロ |
| 正規 client / Leave / probe の input put に `session.new_timestamp()` | temporary ban / strike |

## 設計

1. **`IntentRateLimiter`**: `bind_global` → `by_bind_source`（既定 20 / 10/s）。`allow_bind(source, session_pub, now)` は両バケット原子消費
2. **host gate**: `sample.timestamp().get_id().to_le_bytes()`、無ければ `\0alchemy-bind-unknown`
3. **client**: input / Leave put に timestamp。歴史的 x0040 の順序（Ed25519 → rate → JWT）は維持

## 本 Slice に含めない

| 項目 | 予定 |
|:---|:---|
| mTLS temp・DemoScene ベクタ・metrics+jti | 別 Slice |
| 成功 JWT でのバケット返却 | 採用しない（A のみ） |

## 完了条件

1. 同一 source の unique-pub 洪水は source 予算で止まり、別 source は独立
2. source / pub deny で対応 map が増えない
3. `cargo test -p host-server`（intent_rate）緑
4. Slice / backlog / README / client.md 鮮度一致

## Phase

| Phase | 内容 | 状態 |
|:---|:---|:---|
| 0 | Slice + 目次 | done |
| 1 | intent_rate per-source | done |
| 2 | host gate + client timestamp | done |
| 3 | docs + テスト | done |

← [目次](../README.md)
