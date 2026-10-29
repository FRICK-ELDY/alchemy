# x0040 — Intent rate limit + JWKS hard stale（P1-5）

> **ID:** `x0040` · **slug:** `intent-rate-jwks-hard-stale` · **status:** `done` · **area:** `[host, docs]` · **depends_on:** `[x0021, x0023, x003e]`  
> **優先度:** **P1-5** — per-sub Intent token bucket と host JWKS hard stale TTL

目標: 正当だが高頻度の Intent が host 入力経路を飽和させない（per-`sub` token bucket）。Room Token JWKS は soft SWR のまま、最大 stale（hard TTL）超過後は bind 検証を fail-closed にし、撤去鍵の無限受理を止める。

根拠: [improvement-plan-2026-10-28.md](../../0_doc/evaluation/improvement-plan-2026-10-28.md) **P1-5** · [client.md](../../0_doc/architecture/client.md) · [trust-boundary.md](../../0_doc/architecture/trust-boundary.md)

## 保証する / しない

| 保証する | 保証しない |
|:---|:---|
| steady: Ed25519 + seq evaluate 成功後・`last_seq` commit **前**に per-`sub` bucket（既定 60 / 30/s）。偽 sig で他者 budget を減らせない。rate drop で seq を進めない。Leave（wire）は steady のみ非消費 | client `world_jwks` hard TTL（→ P2-3） |
| bind: Ed25519 後・JWT **前**に **host-global**（既定 20 / 10/s）+ per-`session_pub`（既定 5 / 2/s）。両方足りるときだけ消費。**per-pub 行は消費確定時のみ insert**（global deny で map 増殖しない）。Leave+token も bind bucket 消費（JWT 迂回なし）。JWT/evaluate 失敗で per-pub 破棄。成功 rebind の GC は **新 pub を除外** | Zenoh ACL / temporary ban / strike |
| JWKS soft 60s SWR + **hard 300s**。hard 超過は Room Token verify 拒否 + refresh キック | world 5 分超ダウン中の新規 bind 成功；Prometheus 本格（P3-4） |
| 超過は drop + 集計カウンタ。プロセスは落とさない | 帯域 DoS 全般（transport）；未 bind steady の Ed25519 洪水（sub bucket 無し） |

## 設計

1. **SessionTable evaluate / commit**: `evaluate_*` は副作用なし。gate が rate を挟んでから `commit_*`（seq / bind insert）。`commit_bind` の `removed` は今回の `session_pub` を含めない
2. **IntentRateLimiter**:
   - steady: `allow_steady(sub)` after evaluate；Leave 免除
   - bind: `allow_bind` = global と per-pub を **原子的**に消費（Leave 非免除）；per-pub HashMap は消費成功時のみ insert；失敗 bind は `forget_session_pub`；鍵ローテで外れた **他** pub のみ `forget_session_pubs`
3. **JWKS_HARD_TTL=300s**: `fetched_at` 超過で verify Err（steady・token 空は JWKS 非経由）。hard-stale refresh キックは unknown-kid と同型クールダウン

| Phase | 内容 | 状態 |
|:---|:---|:---|
| 0 | Slice + 目次 | done |
| 1 | intent_rate + spawn_input_gate | done |
| 2 | JWKS hard stale | done |
| 3 | テスト + architecture / improvement-plan | done |
| 4 | evaluate/commit + forged-sig / bind-map 修正 | done |

## 完了条件

- 高頻度 Intent が単体で bucket により drop される
- 偽 sig で他者 steady bucket を減らせない / rate drop で `last_seq` が進まない
- 失敗 bind で `by_session_pub` が単調増加しない；ユニーク pub は global bind で JWT 前に抑制
- hard TTL 超過キャッシュだけでは bind verify が通らない
- `cargo test -p host-server` 緑。保証表・slice・コード一致
- P1-5 を improvement-plan 上 done

← [目次](../README.md)
