# x00d2 — catalog RL + room_key_epoch 軽量 API（P2-2 残）

> **ID:** `x00d2` · **slug:** `catalog-rl-epoch-api` · **status:** `done` · **area:** `[world, host, client, docs]` · **depends_on:** `[x0071, x007e, x00b5, x00d1]`  
> **優先度:** improvement-plan **P2-2**（負債・衛生・world 軽微群）

目標: catalog GET に IP+user rate limit を付け、`GET …/room_key_epoch`（Room Token 認可・PoP 不要）で guest/host の ≈15s probe を Token 再発行から切り離す。scene Grant 照会は **x00b5 SpawnTicket で closed**（本 Slice で assets HEAD は実装しない）。

根拠: [improvement-plan-2026-11-12.md](../../0_doc/evaluation/improvement-plan-2026-11-12.md) **P2-2** · [opus-specific-weaknesses-2026-11-12.md](../../0_doc/evaluation/opus/opus-specific-weaknesses-2026-11-12.md)

---

## 保証する / しない

| 保証する | 保証しない |
|:---|:---|
| `GET /rooms` / `GET /rooms/:id` は IP+user catalog RL（超過 429） | catalog の ACL 評価コストそのものの削減 |
| `GET /rooms/:id/room_key_epoch` は有効 Room Token + room_id 一致で `{room_id,room_key_epoch}`（PoP 不要） | Zenoh epoch bump ヒント / push |
| guest/host の定期 probe は epoch 不変時 Token を再発行しない | PoP nonce（P2-2 残り・別スライス）；probe HTTP 失敗時は `/tokens` へソフトフェイル（連続失敗カウンタなし） |
| scene Grant は x00b5 SpawnTicket で closed と文書上明記 | world→assets 同期 HEAD Grant 照会 |

---

## Phase

| Phase | 内容 | 状態 |
|:---|:---|:---|
| 0 | Slice + 目次 | **done** |
| 1 | catalog RL + config + 429 テスト | **done** |
| 2 | `GET …/room_key_epoch` + RL | **done** |
| 3 | host/guest probe 切替 | **done** |
| 4 | docs / improvement-plan（Grant closed・PoP 残） | **done** |

---

## 完了条件

1. catalog GET が strict limit で 429
2. epoch GET が Room Token で 200、auth JWT のみ／room 不一致は 401/403
3. guest/host が epoch 不変時に `/tokens` を叩かない（単体または結合で確認可能な経路）
4. improvement-plan P2-2: catalog/epoch/Grant closed、PoP nonce 残を明示

## やらない

- PoP nonce
- P2-8 Rooms / Fallback 分割
- KeyStale 無停止（P2-4）

← [目次](../README.md) · [x00d1](x00d1-p2-hygiene-docs-ci-glb.md) · [improvement-plan](../../0_doc/evaluation/improvement-plan-2026-11-12.md)
