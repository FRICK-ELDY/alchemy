# x00d9 — assets HTTP rate limit（P0-3）

> **ID:** `x00d9` · **slug:** `assets-http-rate-limit` · **status:** `done` · **area:** `[assets, shared, docs]` · **depends_on:** `[x004b, x0071, x00b5, x00cd]`  
> **優先度:** improvement-plan **P0-3**（流量）

目標: コア 3 サービスで唯一 RL が無かった assets に path 依存 `alchemy_rate_limit` / `alchemy_client_ip` を導入し、重い入口を IP×sub で抑え、VCS push は sub 同時 1 本で Port プール飢餓を防ぐ。

根拠: [improvement-plan-2026-11-18.md](../../0_doc/evaluation/improvement-plan-2026-11-18.md) **P0-3** · [specific-weaknesses-2026-11-18.md](../../0_doc/evaluation/specific-weaknesses-2026-11-18.md)

---

## 保証する / しない

| 保証する | 保証しない |
|:---|:---|
| `blob_put` / `vcs_negotiate` / `vcs_push` / `spawn_ticket` は操作別 IP×`auth_sub` RL（超過 429 + `retry-after`） | クラスタ横断の共有カウンタ / エッジ WAF |
| `POST …/vcs/…/push` は同一 `sub` 同時 1 本（body 読了前〜`Vcs.push`；2 本目は body 未読のまま 429 `push_in_flight`） | push 以外の Port（blake3）同時本数の公平性 |
| ある `sub` が push を保持中でも、他 `sub` は PushGate / negotiate を塞がれない | 単一 BEAM ノードを超えた fairness |
| `POST /blobs` と `POST /inventory/blobs` は同一 `blob_put` バケット | GET blob / quota / drive / health / JWKS の RL |

---

## Phase

| Phase | 内容 | 状態 |
|:---|:---|:---|
| 0 | Slice + 目次 | **done** |
| 1 | deps / Docker / CI + `Assets.RateLimit` + Plug + config | **done** |
| 2 | `Assets.Vcs.PushGate` + controller 配線 | **done** |
| 3 | 429 / バケット分離 / IP 軸 / Port 飢餓否定系テスト | **done** |
| 4 | moduledoc / ClientIp 保証コメント | **done** |

---

## 完了条件

1. 4 action を strict limit で連打すると 429
2. push 同時 2 本目が `push_in_flight`；他 sub は塞がれない
3. Docker / assets-server-ci が `alchemy_client_ip` / `alchemy_rate_limit` を認識
4. 保証表と Plug / PushGate のコードが一致

## やらない

- P2-1 world `scene/save` RL
- assets GET への RL 拡張
- Hex 新規パッケージ

← [目次](../README.md) · [x00d8](x00d8-lbvh-o-n-refit.md) · [improvement-plan](../../0_doc/evaluation/improvement-plan-2026-11-18.md)
