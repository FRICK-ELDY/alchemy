# x0044 — client world_jwks SWR / hard stale / unknown-kid cooldown

> **ID:** `x0044` · **slug:** `client-world-jwks-cooldown` · **status:** `done` · **area:** `[client, docs]` · **depends_on:** `[x0040, x0026]`  
> **優先度:** **P2-3**

目標: Manifest / HostAttestation 受信パスで JWKS HTTP をしない。host `room_token` と同型の soft SWR・hard fail-closed・unknown kid 冷却 + bg refresh。

根拠: [improvement-plan-2026-10-28.md](../../0_doc/evaluation/improvement-plan-2026-10-28.md) **P2-3**

## 保証する / しない

| 保証する | 保証しない |
|:---|:---|
| `decode_claims` が HTTP しない | audience / typ 検証（呼び出し側） |
| soft 60s は cache で受理 + bg kick | hard 超過中の受理 |
| hard 300s は Err + cooldown kick | bg refresh 失敗時の即時回復 |
| unknown kid は Err + 1s cooldown kick | 偽 kid 連打での CPU（decode 自体） |

## 完了条件

- soft accept / hard reject / unknown-kid cooldown の単体テスト緑
- `main` が初期 refresh 後に `spawn_background_refresh`
- host `room_token` の「client → P2-3」注記を解消

← [目次](../README.md)
