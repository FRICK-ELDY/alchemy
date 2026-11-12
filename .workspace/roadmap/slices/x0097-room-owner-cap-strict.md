# x0097 — Room per-owner 上限の厳密化（並行 overshoot 封じ）

> **ID:** `x0097` · **slug:** `room-owner-cap-strict` · **status:** `done` · **area:** `[world, docs]` · **depends_on:** `[x0071]`  
> **優先度:** improvement **P2-11**（2026-11-05。※旧計画 P2-11＝DemoScene ベクタは [x0079](x0079-demoscene-shared-vector.md)）

目標: owner あたり Room 数上限（既定 **32** / `WORLD_MAX_ROOMS_PER_OWNER`）を、`COUNT`→`INSERT` の best-effort から **quota row の条件付き更新**へ上げ、並行 create でも cap を超えないことを保証する。

根拠: [improvement-plan-2026-11-05.md](../../0_doc/evaluation/improvement-plan-2026-11-05.md) **P2-11** · [gpt-specific-weaknesses-2026-11-05.md](../../0_doc/evaluation/gpt/gpt-specific-weaknesses-2026-11-05.md)（並行 overshoot）· [x0071](x0071-world-rate-limit-room-cap.md)

---

## 保証する / しない

| 保証する | 保証しない |
|:---|:---|
| 同一 `owner_sub` の並行 `create` でも、確定後の保有 Room 数 ≤ `max_rooms_per_owner` | world 全体の Room 総数上限；ルーム内ユーザ数上限 |
| 超過 create は `:owner_room_cap_exceeded` → HTTP 403 `room_cap_exceeded`（x0071 互換） | 既存 overshoot 行の自動削除・強制刈り込み |
| create 成功と quota 増分は同一 TX（insert 失敗時は増分も戻る） | Room 物理 DELETE API（未実装。将来 DELETE は必ず quota 減算と同一 TX） |
| マイグレーションで既存 `rooms` から quota を backfill | Postgres / マルチノード横断の分散カウンタ |
| 既定 cap=32（設定キー・env 名は x0071 のまま） | cap 変更時の既存超過の自動収束 |

---

## 設計（要点）

assets の `QuotaLedger` 条件付き UPDATE（`used+reserved+add <= limit`）と同型。SQLite SERIALIZABLE や DB trigger は採らない（単一 writer でも TOCTOU は残るため、**行ロック相当の条件付き更新**を正とする）。

```
room_owner_quotas(owner_sub PK, room_count INTEGER NOT NULL)
```

```
create(room_id, owner_sub, …)
  # best-effort hint（quota 行の room_count >= cap）→ 超過なら attrs を作らず拒否
  # 門番の正本は以下 TX（mode: :immediate）の条件付き UPDATE
  build attrs（DemoScene / wire_cap / room_key）  # TX 外
  Repo.transaction(immediate):
    INSERT OR IGNORE room_owner_quotas(owner_sub, room_count=0)
    UPDATE … SET room_count = room_count + 1
      WHERE owner_sub = ? AND room_count < ?
    → num_rows == 0 なら :owner_room_cap_exceeded（ensure 直後前提；rollback）
    INSERT rooms …
    → already_exists / storage_error なら rollback（増分も戻る）
```

- 門番の正本は条件付き UPDATE（hint は拒否ホットパスのコスト削減のみ）
- マイグレーション: `INSERT … SELECT owner_sub, COUNT(*) FROM rooms GROUP BY owner_sub`
- 公開 API・エラー atom・403 ボディは変更しない

---

## Phase

| Phase | 内容 | 状態 |
|:---|:---|:---|
| 0 | Slice + roadmap / backlog / improvement リンク | **done** |
| 1 | migration（`room_owner_quotas` + backfill）+ create を TX/条件付き UPDATE へ | **done** |
| 2 | 並行 create 否定系（cap=1 で N 並列→成功ちょうど 1）+ `@moduledoc` / world-server 保証表 | **done** |

---

## 完了条件

1. `max_rooms_per_owner: 1` で同一 owner の create 契約 → 成功 1・残りは `owner_room_cap_exceeded`（DB 上も 1 行）。※ shared sandbox では真のマルチ接続 race は検証しない；条件付き UPDATE は ledger 直叩きで回帰
2. 単発超過・invalid cap・既存 x0071 HTTP テストが緑
3. `World.Rooms` から「may overshoot」「strict serialization … 非保証」を削除し、保証に昇格
4. [world-server.md](../../0_doc/architecture/world-server.md) 保証表に per-owner cap 厳密化を追記
5. improvement P2-11 を done 表記
6. outer `Repo.transaction(..., mode: :immediate)`；scene/鍵は TX 外で用意

## やらない（本 Slice）

- Room DELETE / GC（quota 減算配線は DELETE 導入スライスへ）
- world 全体 Room 総数・CCU / 帯域 quota
- `max_rooms_per_owner` 既定値の変更（32 維持）
- rate limit 変更（x0071 / x0084 のまま）

## 関連

- [x0071-world-rate-limit-room-cap.md](x0071-world-rate-limit-room-cap.md)
- [improvement-plan-2026-11-05.md](../../0_doc/evaluation/improvement-plan-2026-11-05.md)
- assets: `Assets.Blobs` / `QuotaLedger` 条件付き予約

← [目次](../README.md)
