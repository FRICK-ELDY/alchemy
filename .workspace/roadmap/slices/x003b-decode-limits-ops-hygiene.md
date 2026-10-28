# x003b — decode limit + prod S2S + world テスト DB 隔離

> **ID:** `x003b` · **slug:** `decode-limits-ops-hygiene` · **status:** `done` · **area:** `[protocol, host, client, world, auth, docs, ci]` · **depends_on:** `[x003a, x0034, x001e]`

目標: Zenoh protobuf の巨大 payload で OOM しない。手順書どおり prod compose が起動する。日常スタック後も `make ci` の world が緑。

根拠: [improvement-plan-2026-10-28.md](../../0_doc/evaluation/improvement-plan-2026-10-28.md) **P0-4 / P0-5 / P0-6**

正本: [protocol.md](../../0_doc/architecture/protocol.md) · [zenoh-router.md](../../0_doc/architecture/zenoh-router.md) · 各 compose / `.env.prod.example`

## 設計

### P0-4 — decode byte / count limit

| 層 | 内容 |
|:---|:---|
| protocol | message class 別の **pre-decode byte 上限** と **post-decode** entries/strings/assignments 上限（`decode_limits`） |
| host / client | Zenoh sample 受信時に上限超過は drop（counter） |
| テスト | 巨大 protobuf / 過大 entries の否定系 |

### P0-5 — prod compose S2S

| 層 | 内容 |
|:---|:---|
| compose | `AUTH_INTERNAL_TOKEN` / `AUTH_INTERNAL_URL` を auth / world の **prod** compose と `.env.prod.example` に通す |
| CI | PR: `compose config`（S2S 欠落検知）。`/health` は `SMOKE_PROD_UP=1` または workflow_dispatch `up=true`（release ビルドが重いため） |

### P0-6 — world テスト DB 隔離

| 層 | 内容 |
|:---|:---|
| world | test で `WORLD_DATABASE_PATH` を無視（runtime.exs；assets 同型）+ isolation テスト |
| ci | `ci.sh` / `ci-world-local.sh` で `WORLD_DATABASE_PATH=` 強制。Sandbox 後始末 |

| Phase | 内容 | 状態 |
|:---|:---|:---|
| 0 | Slice + 目次 | done |
| 1 | protocol limits + host/client 配線 + テスト | done |
| 2 | prod compose / `.env.prod.example` + smoke | done |
| 3 | world test DB 隔離 + ci.sh | done |
| 4 | 文書（保証表・起動手順） | done |

## 保証する / しない

| 保証する | 保証しない |
|:---|:---|
| class 別上限超過サンプルでプロセスが OOM しない | router 側 max payload / per-peer QoS の完全実装 |
| 手順書どおりの prod compose で auth/world が起動する | Cloudflare Access / WAF as code |
| 開発者 `.env`（本番 DB path）ありでも world `mix test` が汚染されない | assets 以外の全 Elixir アプリの隔離網羅 |

## やらない

- BlobGrant / assets GET 認可の実装（→ P1）
- physics–network worker 分離（→ P1）
- mTLS / room ACL（→ P2）
- cargo-fuzz 本格導入（→ P3）

## 成功条件

1. 巨大 protobuf / 過大 entries を投げても host・client が落ちない（drop）  
2. prod compose（例示 env）で auth / world が `/health` まで到達（`SMOKE_PROD_UP=1` / workflow_dispatch）。PR CI は config で S2S 欠落を検知  

3. 日常 `WORLD_DATABASE_PATH` 設定後も `make ci`（world）緑  
4. 関連単体・否定系テスト緑  

← [目次](../README.md)
