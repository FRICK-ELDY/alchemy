# x0049 — metrics 最小（依存なし・localhost Prometheus text）

> **ID:** `x0049` · **slug:** `minimal-metrics` · **status:** `done` · **area:** `[host, client, assets, docs]` · **depends_on:** `[x0040, x0044, x003f]`  
> **優先度:** **P3-4**

目標: tick lateness / frame age / JWKS stale / CAS を **新規 Cargo・Hex 依存なし**で測り、`127.0.0.1` の手書き Prometheus text（`GET /metrics`）で露出する。

根拠: [improvement-plan-2026-10-28.md](../../0_doc/evaluation/improvement-plan-2026-10-28.md) **P3-4**

## 保証する / しない

| 保証する | 保証しない |
|:---|:---|
| `ALCHEMY_METRICS_ADDR` / `ASSETS_METRICS_ADDR`（または `ASSETS_METRICS=1`→`127.0.0.1:9103`）時のみ metrics 公開 | 未設定時の metrics ポート起動 |
| **専用 loopback bind**（host/client/assets いずれも公開 API ポートに載せない） | Tunnel / 公網スクレイプ；Phoenix Endpoint 上の `/metrics` |
| 固定メトリクス名（下記）・`sub`/`room_id` を label にしない | OTLP / PromEx / Grafana / histogram 本格 |
| JWKS age / hard-stale、tick lateness（+ 閾値超え late counter）、frame age+seen、CAS | 完全な SLO ダッシュボード |

## メトリクス名

| 名前 | サービス |
|:---|:---|
| `alchemy_host_tick_lateness_ms` | host |
| `alchemy_host_tick_late_total` | host（**>5ms** のみ加算） |
| `alchemy_jwks_cache_age_secs{service="host\|client"}` | host / client（client は scrape 時ライブ） |
| `alchemy_jwks_hard_stale_total{service="host\|client"}` | host / client |
| `alchemy_client_frame_seen` / `alchemy_client_frame_age_ms` | client |
| `alchemy_cas_get_total` / `alchemy_cas_get_errors_total` / `alchemy_cas_get_last_ms` | assets |
| `alchemy_cas_put_total` / `alchemy_cas_put_errors_total` / `alchemy_cas_quota_exceeded_total` / `alchemy_cas_put_last_ms` | assets |

## 起動・手動検証

```bash
# host（ZIP 既定 off — 未設定なら metrics スレッドなし）
cd client
ALCHEMY_METRICS_ADDR=127.0.0.1:9101 cargo run -p host-server
curl -s http://127.0.0.1:9101/metrics | grep -E 'alchemy_host_tick_lateness_ms|alchemy_jwks_'

# client
ALCHEMY_METRICS_ADDR=127.0.0.1:9102 cargo run -p client
curl -s http://127.0.0.1:9102/metrics | grep -E 'alchemy_client_frame_|alchemy_jwks_'

# assets — **専用 loopback Bandit**（公開 :4004 には /metrics を載せない）
cd assets-server
ASSETS_METRICS_ADDR=127.0.0.1:9103 mix phx.server
# または ASSETS_METRICS=1 （既定 127.0.0.1:9103）
curl -s http://127.0.0.1:9103/metrics | grep alchemy_cas_
curl -s -o /dev/null -w "%{http_code}" http://127.0.0.1:4004/metrics   # → 404
```

## 単体

```bash
cd client && cargo test -p host-server metrics -- --nocapture
cd client && cargo test -p host-server hard_stale_increments -- --nocapture
cd client && cargo test -p client metrics -- --nocapture
cd client && cargo test -p client hard_stale_increments -- --nocapture
cd assets-server && mix test test/assets/metrics_test.exs test/assets_web/metrics_plug_test.exs test/assets_web/router_metrics_test.exs
```

## 完了条件

- host / client / assets の上記名が **専用 loopback** `/metrics` に出る（有効時）
- 公開 Phoenix ポートに `/metrics` が無い（Tunnel 経由 peer=loopback でも取れない）
- 単体テスト緑（render / age / hard_stale / Plug 404·200）
- 保証表とコード一致

← [目次](../README.md)
