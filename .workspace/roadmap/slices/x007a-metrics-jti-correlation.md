# x007a — auth/world/relay metrics + Room Token `jti` 相関

> **ID:** `x007a` · **slug:** `metrics-jti-correlation` · **status:** `done` · **area:** `[auth, world, relay-agent, host, client, docs]` · **depends_on:** `[x0049, x004e]`

目標: improvement-plan **P2-12** — auth / world / relay-agent に x0049 同型の loopback Prometheus を足し、Room Token `jti` で入室を横断 grep できるようにする。

根拠: [backlog-priority-2026-11-04.md](../backlog-priority-2026-11-04.md) **P3** · [improvement-plan-2026-10-30.md](../../0_doc/evaluation/improvement-plan-2026-10-30.md) **P2-12** · [opus-specific-proposals-2026-10-30.md](../../0_doc/evaluation/opus/opus-specific-proposals-2026-10-30.md)

## 保証する / しない

| 保証する | 保証しない |
|:---|:---|
| `AUTH_METRICS_ADDR` / `WORLD_METRICS_ADDR` / `RELAY_AGENT_METRICS_ADDR`（または `*=1` 既定ポート）時のみ metrics | 未設定時の metrics ポート |
| **専用 loopback bind**（公開 API に `/metrics` を載せない） | Tunnel / 公網スクレイプ；PromEx / OTLP |
| 固定低 cardinality 名（下記）。`jti`/`sub`/`room_id` を label にしない | histogram / SLO ダッシュボード |
| Allocation Token に必須 `room_jti`（Room Token と同じ UUID）。alloc 固有 `jti` は dedupe 用 | 旧 Allocation JWT 互換（短命 TTL） |
| world / relay / host / client が Room `jti` をログに出す（auth は access `jti` 補助） | 分散トレース・ログ集約基盤 |

## メトリクス名

| 名前 | サービス | env 例 |
|:---|:---|:---|
| `alchemy_auth_login_total` / `alchemy_auth_login_errors_total` | auth | `AUTH_METRICS_ADDR=127.0.0.1:9104` または `AUTH_METRICS=1` |
| `alchemy_world_token_issue_total` / `alchemy_world_token_issue_errors_total` | world | `WORLD_METRICS_ADDR=127.0.0.1:9105` または `WORLD_METRICS=1` |
| `alchemy_relay_allocate_total` / `alchemy_relay_allocate_errors_total` | relay-agent | `RELAY_AGENT_METRICS_ADDR=127.0.0.1:9106` または `RELAY_AGENT_METRICS=1` |

## `jti` 相関

| 地点 | ログ |
|:---|:---|
| auth login 成功 | Logger metadata `jti=`（access） |
| world token 発行成功 | metadata `jti=`（Room）+ `room_jti` / `alloc_jti` / `sub` / `room_id` |
| relay allocate 成功 | `room_jti=`（+ alloc `jti=`） |
| host bind 成功 | `sub=` + `jti=` |
| client Manifest Accept | `jti=` + `(gen, rev)` |

## 起動・手動検証

```bash
# auth
cd auth-server
AUTH_METRICS=1 mix phx.server
curl -s http://127.0.0.1:9104/metrics | grep alchemy_auth_login_

# world
cd world-server
WORLD_METRICS=1 mix phx.server
curl -s http://127.0.0.1:9105/metrics | grep alchemy_world_token_

# relay-agent
RELAY_AGENT_METRICS=1 cargo run -p alchemy-relay-agent
curl -s http://127.0.0.1:9106/metrics | grep alchemy_relay_allocate_
```

## 単体

```bash
cd auth-server && mix test test/auth/metrics_test.exs test/auth_web/metrics_plug_test.exs test/auth_web/router_metrics_test.exs
cd world-server && mix test test/world/metrics_test.exs test/world_web/metrics_plug_test.exs test/world_web/router_metrics_test.exs test/world/allocation_token_test.exs
cd relay-agent && cargo test metrics -- --nocapture
cd relay-agent && cargo test room_jti -- --nocapture
```

## 本 Slice に含めない

| 項目 | 予定 |
|:---|:---|
| scene despawn | 別 Slice |
| PromEx / OTLP / `alchemy_metrics` 共有 Hex | 人に回す / しない |
| metrics label に `jti` | しない（cardinality） |

## 完了条件

1. auth / world / relay-agent の上記名が **専用 loopback** `/metrics` に出る（有効時）
2. 公開 Phoenix ポートに `/metrics` が無い
3. Allocation JWT に `room_jti` 必須；relay が検証
4. world→relay→host→client で Room `jti=` を grep 可能
5. 単体テスト緑・Slice / backlog / README 鮮度一致

## Phase

| Phase | 内容 | 状態 |
|:---|:---|:---|
| 0 | Slice + 目次 | done |
| 1 | Allocation `room_jti` | done |
| 2 | loopback metrics | done |
| 3 | jti ログ相関 | done |
| 4 | テスト・鮮度 | done |

← [目次](../README.md)
