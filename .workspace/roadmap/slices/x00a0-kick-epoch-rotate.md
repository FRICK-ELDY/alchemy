# x00a0 — kick / friendship → Room key epoch（P1-1）

> **ID:** `x00a0` · **slug:** `kick-epoch-rotate` · **status:** `done` · **area:** `[world, auth, docs]` · **depends_on:** `[x007e, x0033, x0034]`  
> **優先度:** improvement-plan P1-1（MVCC Phase 1 必須ゲート）

目標: owner が `POST …/room_key/rotate` で即 epoch を上げられるようにし、auth の friendship / follow 解除後に social visibility Room を CAS ローテする。旧参加者は新 epoch の平文を旧鍵で読めない。

根拠: [improvement-plan-2026-11-12.md](../../0_doc/evaluation/improvement-plan-2026-11-12.md) P1-1 · Opus/GPT「kick / friendship 解除の即時 revoke」

---

## 保証する / しない

| 保証する | 保証しない |
|:---|:---|
| `POST /api/v1/rooms/:id/room_key/rotate` は owner のみ・`:room_visibility` RL。成功で `room_key_epoch` +1 + 新鍵 + grace | Room 単位 deny list / ban UI；Moderator ロール |
| auth `unfriend` / friendship を消す `unfollow` 成功後、`POST /internal/v1/relation_changed` で world に通知。owner が当事者で `needs_social?` な Room を CAS ローテ | 通知失敗時の再試行キュー；既発行 Room Token の JWT 即時失効（TTL まで ACL は残るが **新 epoch 平文は旧鍵で読めない**） |
| 否定系: rotate / relation 後、旧 `room_key` + 新 epoch では AEAD open 不可（protocol `wrong_epoch` / key 不一致と world 統合で確認） | push ヒント無しの 15s Token 再取得以外の即時配信 |

---

## Phase

| Phase | 内容 | 状態 |
|:---|:---|:---|
| 0 | Slice + 目次 | **done** |
| 1 | HTTP rotate + RL + controller テスト | **done** |
| 2 | world internal relation_changed + Rooms 一括ローテ | **done** |
| 3 | auth → world notify（unfriend / unfollow） | **done** |
| 4 | 否定系 + 保証表 | **done** |

## 完了条件

1. owner `POST …/room_key/rotate` → 200 + epoch bump；非 owner → 403。
2. friends Room で unfriend/relation_changed 後、旧鍵では新 epoch 平文が読めない。
3. architecture（world-server / auth）保証表が本 Slice と一致。
