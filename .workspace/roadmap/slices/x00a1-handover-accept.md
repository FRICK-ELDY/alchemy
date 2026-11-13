# x00a1 — handover 候補 accept 二段階（P1-4）

> **ID:** `x00a1` · **slug:** `handover-accept` · **status:** `done` · **area:** `[world, client, docs]` · **depends_on:** `[x0080, x009e, x00a0]`  
> **優先度:** improvement-plan P1-4（MVCC Phase 1 必須ゲート）

目標: Presence 先頭への一発 lease 付与をやめ、**nominate（候補記録 + dormant）→ accept（CAS で active_host + lease）** の二段階にする。候補不在 / null は即 dormant。非 accept 候補に lease が付かない。

根拠: [improvement-plan-2026-11-12.md](../../0_doc/evaluation/improvement-plan-2026-11-12.md) P1-4 · x009e 非保証「candidate opt-in」

---

## 保証する / しない

| 保証する | 保証しない |
|:---|:---|
| `POST …/handover` with `target_sub`: `handover_candidate_sub` を記録し **即 dormant**（`active_host_sub` は null）。null → dormant + candidate クリア。pending 中の再 `handover(other)` は **candidate 差し替え**（cancel 必須ではない） | Presence / Server Operator の world 検証 |
| `POST …/handover/accept`: caller == candidate のときだけ CAS で active + lease + overlay clear + candidate クリア | 署名付き HostDelegation JWT |
| 否定系: nominate のみでは host Token 不可；非 candidate の accept → 403；並行 accept は一方のみ成功；pending 中の owner claim / takeover 横取り → 403/409 | accept TTL の長寿命永続（短命カラムで十分） |
| GET / lease_view は期限切れ candidate を null 投影；sweeper が列を消す | — |

### 列

- `handover_candidate_sub`（nullable text）
- `handover_candidate_expires_at`（nullable utc；既定 ≈ host lease TTL）

---

## Phase

| Phase | 内容 | 状態 |
|:---|:---|:---|
| 0 | Slice + 目次 | **done** |
| 1 | migration + Rooms nominate/accept CAS | **done** |
| 2 | HTTP accept + session summary に candidate | **done** |
| 3 | client: leave nominate + Waiting accept | **done** |
| 4 | 否定系 + 保証表 | **done** |

## 完了条件

1. nominate 後 `active_host_sub` は null；candidate のみセット。
2. 非 candidate accept → 403；accept 後のみ host Token 可。
3. client Leave は Presence 一発 lease をしない（nominate または dormant）。
