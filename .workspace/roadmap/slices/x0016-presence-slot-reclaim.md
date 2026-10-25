# x0016 — Presence スロット解放（Intent TTL）

> **ID:** `x0016` · **slug:** `presence-slot-reclaim` · **status:** done · **area:** `[host, protocol, docs]` · **surface:** `[desktop, vr]` · **depends_on:** `[x0015]`

目標: client 切断・クラッシュ・Intent 沈黙後に **user_root スロットを解放・再利用**する。検知は既存 `InputEvent`（30Hz）の **無音 TTL（3s）**のみ。解放時は Presence を bump し、該当 entity を `RenderFrame.removes` で通知。保持中 Grab は解放し user_root を spawn へ戻す。（完了）

正本: [client.md](../../0_doc/architecture/client.md) · [protocol.md](../../0_doc/architecture/protocol.md)

## 設計方針

| 層 | 役割 |
|:---|:---|
| protocol | スキーマ据え置き。`PROTOCOL_PIN` のみ `0.7.0-x0016`。保証にスロット解放を追加 |
| host | occupied ごとに `last_input`。TTL 超過で reclaim → Presence + frame removes + sim reset |
| client | 変更なし（Presence 欠落で `local_user_root` unset、`removes` で entity 削除は x0015 済み） |
| Identity | 引き続き Token `sub` のみ。明示 Leave / liveliness なし |

| Phase | 内容 | 状態 |
|:---|:---|:---|
| 0 | Slice 文書 + 目次 | done |
| 1 | protocol pin + 正本 docs | done |
| 2 | host reclaim + sim reset + frame removes | done |
| 3 | unit + smoke + README | done |

## Presence / 解放契約

- TTL: **3 秒**（`PRESENCE_SLOT_TTL`）。client `INPUT_HZ=30` 前提
- 同一 `sub` が TTL 前に再 Intent → 既存スロット継続
- 解放後の再割当は first-free（entity は変わることがある）
- 解放時: `PresenceState.revision` bump、`assignments` から除外、`RenderFrame.removes` に entity、Grab 解放、user_root → `USER_SPAWN[slot]`
- `PROTOCOL_PIN`: `alchemy-protocol/0.7.0-x0016`

## 保証する / しない

| 保証する | 保証しない |
|:---|:---|
| occupied が Intent 無音 3s 超で解放・再利用可 | 明示 Leave・即時解放 |
| 解放時 Presence revision bump + 周期 republish | JWT 失効の能動 revoke |
| 解放 entity を `RenderFrame.removes` で通知 | 予測補間・アバター・Steal |
| 保持中 Grab 解放 + user_root spawn リセット | 5 人以上・`ROOM_ID` 本配線 |

## やらない（後続）

- `ROOM_ID` → Zenoh キー本配線 → **[x0017](x0017-room-id-zenoh-keys.md)**
- client アカウント殻 → **[x0018](x0018-client-auth-session.md)**
- 明示 Leave / Zenoh liveliness
- Steal / 投げ / アバター GLB
- launcher / verifier / 予測補間

## 起動（同一マシン smoke）

```bash
zenohd -c zenoh-router/config.json5
cd client && cargo run -p host-server

ALCHEMY_FORCE_DESKTOP=1 AUTH_USER=alice cargo run -p client
# alice 終了後 ~3s でスロット解放 → bob が入れる
ALCHEMY_FORCE_DESKTOP=1 AUTH_USER=bob cargo run -p client
```

## 実装メモ

- host: `PresenceIntents::last_input` + `reclaim_stale`
- sim: `release_slot(slot)` — Grab + spawn reset
- smoke: `.scripts/smoke-presence-slots.sh`（pin / banner を x0016 へ）

← [目次](../README.md)
