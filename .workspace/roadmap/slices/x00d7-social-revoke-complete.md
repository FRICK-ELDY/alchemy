# x00d7 — social revoke 完了（P0-1）

> **ID:** `x00d7` · **slug:** `social-revoke-complete` · **status:** `done` · **area:** `[auth, world, docs]` · **depends_on:** `[x00a0, x0035]`  
> **優先度:** improvement-plan 2026-11-18 **P0-1**

目標: 全 unfollow で durable `relation_changed` を出し、auth が 2hop 影響 owner を mutation 前に計算して outbox に載せ、world が epoch CAS の **成功後** に `event_id` を record して冪等化する。world 一時停止後も outbox 再送で閉じる。旧鍵では新 epoch 平文を読めない。

根拠: [improvement-plan-2026-11-18.md](../../0_doc/evaluation/improvement-plan-2026-11-18.md) P0-1 · [specific-weaknesses-2026-11-18.md](../../0_doc/evaluation/specific-weaknesses-2026-11-18.md) social revoke -3 · x00a0 部分完了の残件

---

## 保証する / しない

| 保証する | 保証しない |
|:---|:---|
| 成功した **全** `unfollow` / `unfriend` が同一 DB TX で `relation_outbox` 行を残す | kick `denied_subs` / ban UI（P1-2） |
| outbox Dispatcher が 2xx まで再送（claim + SKIP LOCKED）。world は rotate **成功後** に `event_id` を記録（失敗時は再送可） | 既発行 Room Token JWT の即時失効（TTL；**新 epoch 平文は旧鍵で読めない**）；`delivered` outbox の無期限保管（既定 7 日 purge） |
| POST body の `affected_owner_subs`（2hop 安全上界）に属する owner の `needs_social?` Room を epoch CAS | world→auth グラフ再照会；影響集合の厳密最小性（過回転可） |
| 否定系: 通常 unfollow / FoF owner / 再送 / **record 成功後**の同一 `event_id` 再 POST で epoch が二重に進まない | push ヒント無しの Token 再取得以外の即時配信；record **前**の並行配送による過回転 |

---

## Phase

| Phase | 内容 | 状態 |
|:---|:---|:---|
| 0 | Slice + 目次 | **done** |
| 1 | auth `relation_outbox` + Dispatcher + WorldNotify 拡張 | **done** |
| 2 | Social 全 unfollow/unfriend → TX outbox | **done** |
| 3 | world `relation_events` + affected owners CAS | **done** |
| 4 | 否定系テスト + architecture 保証表 | **done** |

## 完了条件

1. friendship 無し unfollow 後、followee の `followers` Room が epoch+1。
2. Carol–Bob revoke で Alice の `friends_plus` Room が `affected_owner_subs` 経由で回転。
3. world 一時失敗後、outbox 再送で delivered。`relation_events` に **record 済み**の同一 `event_id` 再 POST では epoch が二重に進まない（record 前の並行配送は過回転可）。
4. architecture（auth / world）保証表が本 Slice と一致。
