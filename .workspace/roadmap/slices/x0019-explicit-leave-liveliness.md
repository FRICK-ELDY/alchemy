# x0019 — 明示 Leave / Zenoh liveliness

> **ID:** `x0019` · **slug:** `explicit-leave-liveliness` · **status:** `done` · **area:** `[host, client, protocol, docs]` · **surface:** `[desktop, vr]` · **depends_on:** `[x0018]`

目標: logout / 明示退室で **即時** Presence スロット解放。プロセス切断は **Zenoh liveliness** で検知して同様に reclaim。Intent TTL 3s は両方失敗時のフォールバックとして維持する。（完了）

正本: [protocol.md](../../0_doc/architecture/protocol.md) · [client.md](../../0_doc/architecture/client.md)

## 設計方針

| 層 | 役割 |
|:---|:---|
| protocol | `InputEvent.leave` + `liveliness_key` / `liveliness_wildcard`。`PROTOCOL_PIN` → `0.9.0-x0019` |
| host | `reclaim_sub` 共通化。Leave 受信・liveliness Delete → 即時解放。TTL フォールバック維持 |
| client | join 後 liveliness token。logout / leave で Leave put + token undeclare |
| Identity | 引き続き Token `sub`。liveliness キー末尾も同一 `sub` |

| Phase | 内容 | 状態 |
|:---|:---|:---|
| 0 | Slice 文書 + 目次 | done |
| 1 | protocol pin + Leave + liveliness ヘルパ + 正本 docs | done |
| 2 | host: reclaim_sub + Leave + liveliness subscriber | done |
| 3 | client: liveliness token + Leave on logout | done |
| 4 | smoke + README | done |

## 契約

| 経路 | 信号 | host 動作 |
|:---|:---|:---|
| 明示 Leave | `InputEvent { leave: true, room_token }` を 1 回 put | verify → `reclaim_sub(sub)`（移動適用なし） |
| Liveliness | `alchemy/dev/room/{room_id}/alive/{sub}` の Delete | キー末尾 `sub` → `reclaim_sub` |
| TTL | Intent 無音 3s（x0016） | フォールバック reclaim |

- 解放時: Presence `revision` bump、`assignments` 除外、`RenderFrame.removes`、Grab 解放、user_root → spawn（既存 reclaim 経路）
- `PROTOCOL_PIN`: `alchemy-protocol/0.9.0-x0019`
- logout は Zenoh session を閉じないため、**Leave put + liveliness undeclare の両方**が必要

## 保証する / しない

| 保証する | 保証しない |
|:---|:---|
| logout / 明示 Leave で TTL を待たず Presence 除外 | 5 人以上・動的スケール |
| liveliness 消失でも同様の即時 reclaim | JWT 能動 revoke / world Token 失効連動 |
| Intent TTL 3s をフォールバックとして維持 | liveliness の WAN / リレー ACL |
| Desktop + VR の logout 経路から Leave 送信 | VR 専用ログイン UI・仮想キーボード |

## やらない（後続）

- Steal / 投げ / アバター GLB
- launcher / verifier / 予測補間
- メール確認・パスワードリセット UI
- 公式リレー ACL・テナント名前空間

## 起動（同一マシン smoke）

```bash
zenohd -c zenoh-router/config.json5
cd client && cargo run -p host-server

ALCHEMY_FORCE_DESKTOP=1 AUTH_USER=alice cargo run -p client
# Home → logout → host が ~1s 以内に reclaimed（3s TTL を待たない）

# クラッシュ相当: client を kill → liveliness Delete → 同様に即時 reclaim
```

## 実装メモ

- protocol: `leave = 11`；`liveliness_key(room_id, sub)` / `liveliness_wildcard(room_id)`
- host: `PresenceIntents::reclaim_sub` + `pending_removes`；liveliness sub on `alive/**`
- client: `SharedState` に liveliness token；`leave_room` が Leave put → undeclare → local clear
- smoke: `.scripts/smoke-leave-liveliness.sh`

← [目次](../README.md)
