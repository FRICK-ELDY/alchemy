# x001e — P0 信頼境界（Tunnel prod / host_control / seed / assets CI）

> **ID:** `x001e` · **slug:** `p0-trust-boundary` · **status:** `active` · **area:** `[auth, world, assets, host, client, relay, docs, ci]` · **depends_on:** `[x001d]`

目標: 2026-10-26 評価の **P0**（公開面の穴）を塞ぐ。コード量少・被害大の項目を先に潰し、次回評価の正味を押し上げる。

根拠: [improvement-plan-2026-10-26.md](../../0_doc/evaluation/improvement-plan-2026-10-26.md)

## 設計方針

| ID | 内容 |
|:---|:---|
| P0-1 | Tunnel 手順は release / `docker-compose.prod.yml` のみ。auth `/dev/*` は ingress 404 |
| P0-2 | Zenoh `host_control` は既定オフ。`ALCHEMY_HOST_CONTROL=1` で opt-in。WAN から止められない |
| P0-3 | release バイナリに seed `alice`/`Password1` 経路なし（debug のみ） |
| P0-4 | assets `runtime.exs` は test で CAS env を無視。CI は `ASSETS_CAS_BACKEND=local` 強制 |

| Phase | 内容 | 状態 |
|:---|:---|:---|
| 0 | Slice 文書 + 目次 | done |
| 1 | host_control 既定無効 + client/host 配線 | done |
| 2 | seed fail-closed（`#[cfg(debug_assertions)]`） | done |
| 3 | assets test 隔離 + ci.sh / local script | done |
| 4 | compose.prod + edge / manual 更新 + `/dev/*` 404 | done |

## 保証する / しない

| 保証する | 保証しない |
|:---|:---|
| 公網手順が prod release を前提にし、mailbox が届かない | Cloudflare Access / WAF as code |
| 既定 host が Zenoh shutdown を購読しない | 署名付き control / named pipe launcher |
| release で未設定 seed ログインしない | Friendship / OIDC |
| `.env` ありでも `make ci-assets` が R2 に触れない | MinIO での S3 統合テスト |

## やらない（後続）

- P1 Token refresh / JWKS bg / magnitude / host role（→ x001f）
- P2 liveliness / WAN TLS / session binding
- Manifest 駆動（P4）

## 成功条件

1. `ALCHEMY_HOST_CONTROL` 未設定の host は `host_control` を subscribe しない  
   → `alchemy_protocol::host_control_zenoh_enabled_from` 単体テスト
2. release 相当（`allow_seed=false`）で未設定 credentials がエラー  
   → `host-server` `resolve_auth_login_credentials` 単体テスト
3. Tunnel runbook が `docker-compose.prod.yml` を指示し、`/dev/mailbox` が 404
4. 開発者 `.env`（R2）ありでも assets `mix test` は local CAS  
   → `Assets.TestEnvIsolationTest`

## 起動メモ

```bash
# host: Zenoh control は触らない（既定）
cargo run -p host-server

# ローカルだけ古い stub が必要なとき
ALCHEMY_HOST_CONTROL=1 cargo run -p host-server
```
