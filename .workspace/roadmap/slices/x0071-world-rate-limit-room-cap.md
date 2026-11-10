# x0071 — world rate limit + Room per-owner cap

> **ID:** `x0071` · **slug:** `world-rate-limit-room-cap` · **status:** `done` · **area:** `[world, auth, docs, ci]` · **depends_on:** `[x0043, x0021, x0028]`

目標: `Auth.RateLimit` の ETS 固定窓コアを `shared/alchemy_rate_limit` に切り出し、world の `:user_auth` API に IP+user 429 を付け、Room は owner あたり上限で create を止める。BLAKE3 ストリーミング（P3-7）は含めない。

根拠: [backlog-priority-2026-11-04.md](../backlog-priority-2026-11-04.md) **P1-4** · [improvement-plan-2026-10-30.md](../../0_doc/evaluation/improvement-plan-2026-10-30.md) **P1-3**

## 保証する / しない

| 保証する | 保証しない |
|:---|:---|
| `:user_auth` の create / token / visibility は IP+user で 429 | ~~host Room Token ルートの RL~~ → [x0084](x0084-host-token-rate-limit.md) |
| owner あたり Room 数は設定上限で create 拒否（403） | 並行 insert の厳密直列化・DB trigger |
| auth 既存 RL 挙動は API 互換のまま | BLAKE3 / seal-hash 常駐化 |

## 設計（要点）

- `shared/alchemy_rate_limit`（MPL-2.0 path）に GenServer + ETS 固定窓を集約
- auth / world は thin wrapper（table・telemetry prefix・config は各アプリ）
- world plug は `:user_auth` 後段のみ
- `World.Rooms.create/3` で count → insert；`owner_sub` インデックス

## 完了条件

1. shared 単体テスト緑 + CI workflow
2. auth 既存 RL / plug テスト緑（API 互換）
3. world `:user_auth` 連打で 429
4. per-owner cap 超過で 403 `room_cap_exceeded`
5. Elixir format + auth/world/shared test 緑

## Phase

| Phase | 内容 | 状態 |
|:---|:---|:---|
| 0 | Slice + roadmap / backlog | done |
| 1 | `shared/alchemy_rate_limit` + CI | done |
| 2 | auth thin wrapper | done |
| 3 | world HTTP RL | done |
| 4 | Room per-owner cap | done |

← [目次](../README.md)
