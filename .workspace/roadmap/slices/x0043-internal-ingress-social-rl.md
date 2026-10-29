# x0043 — `/internal/*` ingress 404 + Social rate limit

> **ID:** `x0043` · **slug:** `internal-ingress-social-rl` · **status:** `done` · **area:** `[auth, docs]` · **depends_on:** `[x0033, x001e]`  
> **優先度:** **P2-2**

目標: 公網 Tunnel から auth の `/internal/*` に届かないようにし、Follow / Friendship mutate に rate limit を付ける。

根拠: [improvement-plan-2026-10-28.md](../../0_doc/evaluation/improvement-plan-2026-10-28.md) **P2-2**

## 保証する / しない

| 保証する | 保証しない |
|:---|:---|
| auth ホストの `/internal/*` は cloudflared ingress で 404（`/dev/*` と同型） | Cloudflare Access |
| Social mutate（follow / friend 系）は IP + user 軸で 429 | アクション別細粒度バケット |
| LAN / Docker 内の world → auth S2S（直叩き）は従来どおり | world 側 `/internal` 公開 |

## 設計

1. [`edge/cloudflared/config.yml.example`](../../../edge/cloudflared/config.yml.example) に `path: /internal/*` → `http_status:404`
2. social scope: `pipe_through [:api, :authenticated, :rate_limited]`
3. `AuthWeb.Plugs.RateLimit` に `:social_mutate`（`/api/v1/social/**`）

## 完了条件

- 手順書どおり `/internal/v1/access` の公網確認が 404（**稼働中 `config.yml` への反映＋再起動は運用確認**；example だけでは不足）
- Social mutate 連打で 429 + `retry-after`（user 軸・IP 軸の単体テストあり）
- `mix test`（rate_limit / social）緑

← [目次](../README.md)
