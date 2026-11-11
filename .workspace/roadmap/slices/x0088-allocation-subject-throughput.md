# x0088 — Allocation 主体別 throughput（sub/IP 短長窓）

> **ID:** `x0088` · **slug:** `allocation-subject-throughput` · **status:** `done` · **area:** `[relay-agent, ci, docs]` · **depends_on:** `[x0083, x007a]`  
> **優先度:** 評価 P1-5（正規 jti 連打でも OpenSSL CPU を飽和させない）

目標: relay-agent `POST /v1/allocate` に **検証済み `sub` + peer IP** の短窓・長窓固定窓 bucket を入れ、JWT 検証後・openssl 前に 429 する。throttle を既存 loopback metrics に出す。

根拠: [improvement-plan-2026-11-05.md](../../0_doc/evaluation/improvement-plan-2026-11-05.md) **P1-5** · [gpt-specific-weaknesses-2026-11-05.md](../../0_doc/evaluation/gpt/gpt-specific-weaknesses-2026-11-05.md)（Allocation 時間窓 RL なし）

---

## 保証する / しない

| 保証する | 保証しない |
|:---|:---|
| allocate: JWT 成功後に **sub** と **peer IP** 各々で短窓・長窓 epoch 固定窓を消費；超過は **429** + `Retry-After`（openssl 前） | 分散共有カウンタ（単一プロセス） |
| 既定: 短窓 10s（sub≤3 / IP≤8）、長窓 60s（sub≤10 / IP≤30）；env で調整可（逆転は sanitize） | world 正経路 CSR 発行の同型 RL（別 Slice） |
| metrics: `alchemy_relay_allocate_throttled_total{axis="sub\|ip"}`（低 cardinality；`errors_total` にも加算） | `sub` / IP を metrics label に載せること |
| 既存: 接続32・同時8・`cert_issue_lock`・jti 一回限りは維持 | token-bucket 滑らかさ；分散 DoS 全般；`X-Forwarded-For` |
| tunnel/loopback では IP≈`127.0.0.1` → IP 軸はプロセス全体上限；主体別は **sub** | 実クライアント IP |

---

## 設計（要点）

| 軸 | 内容 |
|:---|:---|
| 順序 | JWKS 検証 → **throughput hit** → jti reserve → `sign_csr` |
| 鍵 | `claims.sub`（検証済み）と `peer_addr().ip()`（取得失敗は 503；`0.0.0.0` に落とさない） |
| 窓 | Elixir `AlchemyRateLimit` と同型の **epoch** 固定窓（`unix_ms / period_ms`；新規 crate なし） |
| 失敗 | 拒否軸は increment 済み（Elixir 同型）。後段拒否時は前段を rollback（long リトライが short を食わない） |
| 地図 | 窓あたりキー上限 256 + LRU（アクティブ sub/IP を優先保持） |
| sanitize | 短窓/短上限が長より大きい誤設定は **短側を引き下げ**（長を緩くしない） |

---

## Phase

| Phase | 内容 | 状態 |
|:---|:---|:---|
| 0 | Slice + roadmap / backlog / improvement | **done** |
| 1 | `throughput` + allocate 配線 + 429 | **done** |
| 2 | metrics + 単体 / HTTP テスト | **done** |
| 3 | README 保証表 | **done** |

---

## 完了条件

1. 同一 sub（異なる jti）短窓超連打 → 429；別 sub は通る
2. IP 軸単独でも超過で 429
3. inflight 503 / jti 401 と共存
4. metrics に throttle カウンタ；improvement P1-5 done

## やらない（本 Slice）

- world `MtlsIssuer` / room_token の短長窓（正経路は別）
- Intent / host-token RL の再実装
- Spot-Check / market

## 関連

- [x0083-relay-agent-csr-only.md](x0083-relay-agent-csr-only.md)
- [x0084-host-token-rate-limit.md](x0084-host-token-rate-limit.md)
- [improvement-plan-2026-11-05.md](../../0_doc/evaluation/improvement-plan-2026-11-05.md)

← [目次](../README.md)
