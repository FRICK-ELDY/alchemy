# x0068 — relay 契約（NotAfter ≈ TTL / JWKS·body bound / CI）

> **ID:** `x0068` · **slug:** `relay-mtls-ttl-bound-ci` · **status:** `done` · **area:** `[relay, world, host, client, ci, docs]` · **depends_on:** `[x004e, x0066]`

目標: 短命 mTLS の **証明書 NotAfter を API TTL（300s）と一致**させ、relay-agent の Allocation 受理を **JWKS hard / body 上限 / 発行 concurrency / jti dedupe** で bounded にし、**CI・cargo deny・契約テスト**に載せる（improvement-plan P0-1 / P0-2 / P1-1）。

## 保証する / しない

| 保証する | 保証しない |
|:---|:---|
| world `MtlsIssuer` と relay-agent の client cert **NotAfter ≈ 300s**（本番 WAN 前提） | CRL / OCSP；退室直後の即時失効 |
| API soft TTL と NotAfter の一致；guest は refresh 後 Zenoh reopen | **host mid-flight Zenoh reopen**（PEM 更新のみ） |
| JWKS hard TTL、**per-kid** unknown-kid cooldown（上限 256・満杯 FIFO）、body 上限、接続 ≤ 32、allocate in-flight ≤ 8、jti は issue 前 reserve（失敗・応答失敗時 release） | Agent CSR 化（P0-4）；WAN compose E2E（P1-2） |
| `make ci` / GitHub Actions に relay-agent（fmt/clippy/test/deny）・テスト >0 | launcher deny；zenohd 本体の変更 |

## 完了条件

1. 発行直後の NotAfter が now+300s ± 余裕（world ExUnit + relay-agent テスト）
2. oversize CL/body → 413；stale JWKS hard / unknown kid → 401；同一 jti → 拒否；同時発行超過 → 503
3. `ci.sh relay`（または `all`）と `relay-agent-check` job が緑；cargo deny 対象

## Phase

| Phase | 内容 | 状態 |
|:---|:---|:---|
| 0 | Slice + 目次 | done |
| 1 | NotAfter = 300s（world + agent + client 定数） | done |
| 2 | Agent 受理契約 + lib 分割 + テスト | done |
| 3 | CI / deny / Makefile | done |
| 4 | 保証表・手順書から 1day NotAfter を除去 | done |

## 実装メモ

- TTL は Room Token 既定（`room_token_ttl_seconds: 300`）に揃える
- OpenSSL 3 系の `-not_after YYYYMMDDHHMMSSZ`（UTC）を前提；失敗は fail-closed
- 新 Cargo / Hex 依存は追加しない

← [目次](../README.md)
