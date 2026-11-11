# x0084 — host-token 操作別 rate limit

> **ID:** `x0084` · **slug:** `host-token-rate-limit` · **status:** `done` · **area:** `[world, docs]` · **depends_on:** `[x0071, x0081, x006b]`  
> **優先度:** 評価 P1-1（host Room Token 系 HTTP の門番衛生）

目標: `host_keys` / `manifest_seal` / scene / PlaceRead を **操作別** に IP + `room_id|jti` で 429 し、盗んだ／連打する host Room Token で seal Port や DB が増幅しないようにする。

根拠: [improvement-plan-2026-11-05.md](../../0_doc/evaluation/improvement-plan-2026-11-05.md) **P1-1** · [gpt-specific-weaknesses-2026-11-05.md](../../0_doc/evaluation/gpt/gpt-specific-weaknesses-2026-11-05.md)（host-token scope が RL 外）· [x0071](x0071-world-rate-limit-room-cap.md) 非保証「host Room Token ルートの RL」

---

## 保証する / しない

| 保証する | 保証しない |
|:---|:---|
| host-token scope（下記）は操作別バケットで IP + **Bearer claims** の `room_id\|jti` 固定窓 429 | auth-JWT scope の再設計；Allocation / Relay の RL |
| `host_seal`（`host_keys` + `manifest_seal`）/ `host_scene` / `place_read`（発行 + hashes）で **action 別** 上限分離 | 操作内のさらに細かいサブリソース別上限 |
| jti 軸キーは **claims の `room_id` + `jti`**（path の room_id 回転では分割されない） | path と claims の room 不一致時の早期 403 前コスト（PoP 検証前に RL は数える） |
| `place_read` 家族の **共有床は IP**。発行（Room Token `jti`）と hashes（PlaceRead JWT `jti`）の **jti 軸は連動しない**（別 Bearer） | 発行→PlaceRead への jti 相関・単一 Token 予算 |
| Bearer 欠落・peek 失敗時も **IP 軸は適用** | JWT 署名検証前の jti 真正性；PoP 検証コストの完全排除 |
| 正規経路は既定上限内で通る（test は高い天井） | 分散ノード横断の厳密共有カウンタ；Redis 等外部ストア |

---

## 設計（要点）

| action | ルート | 軸 |
|:---|:---|:---|
| `host_seal` | `POST …/host_keys` · `POST …/manifest_seal` | IP · claims `room_id\|jti` |
| `host_scene` | `GET …/scene` · `POST …/scene/entries` · `DELETE …/scene/entries/:id` | 同上 |
| `place_read` | `POST …/place_read` · `GET …/place_hashes` | IP 共有；jti は各 Bearer ごと |

- 既存 `WorldWeb.Plugs.RateLimit` + `World.RateLimit`（`AlchemyRateLimit`）を拡張。router の host-token scope に同じ plug を載せる
- `jti` は `Joken.peek_claims`（検証前）。軸キーは claims `"#{room_id}|#{jti}"`（`room_id` 欠落時は `jti` alone）
- 既定上限（prod/config）: seal を scene / PlaceRead より厳しく（Port / 署名コスト）

---

## Phase

| Phase | 内容 | 状態 |
|:---|:---|:---|
| 0 | Slice + 目次 / backlog | **done** |
| 1 | config + RateLimit buckets + plug 経路判定 | **done** |
| 2 | router に host-token pipeline | **done** |
| 3 | 否定系テスト（操作別・IP/jti） | **done** |
| 4 | 保証表・improvement 追記 | **done** |

---

## 完了条件

1. 同一 host Token で `manifest_seal`（または `host_keys`）を上限超連打 → 429 + `retry-after`
2. scene 連打と seal 連打は **別バケット**（一方を飽和させても他方は通る）
3. PlaceRead 発行連打で 429
4. Bearer 無し連打でも IP 軸で 429
5. architecture 保証表に host-token RL を明記；world test 緑

## やらない（本 Slice）

- GLB decode budget（P1-2）
- assets test hermetic（P1-3）
- ~~Allocation 主体別 throughput（P1-5）~~ → [x0088](x0088-allocation-subject-throughput.md)
- seal Port の PartitionSupervisor プール（P2-1）

## 関連

- [x0071-world-rate-limit-room-cap.md](x0071-world-rate-limit-room-cap.md)
- [x0081-control-plane-pop.md](x0081-control-plane-pop.md)
- [improvement-plan-2026-11-05.md](../../0_doc/evaluation/improvement-plan-2026-11-05.md)

← [目次](../README.md)
