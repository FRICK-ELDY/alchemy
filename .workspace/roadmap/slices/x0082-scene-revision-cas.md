# x0082 — scene revision CAS（楽観 concurrent mutation）

> **ID:** `x0082` · **slug:** `scene-revision-cas` · **status:** `done` · **area:** `[world, host, docs]` · **depends_on:** `[x006d, x007b, x0081]`  
> **優先度:** improvement P0-3（canonical scene の同時更新欠落）

目標: host の scene append/delete を **`expected_revision` 必須の楽観 CAS** にし、競合時は **409** を返して host が GET 再取得後に有限回再試行する。同時 append+delete でも両 mutation が最終 DB に残ることを保証する。

根拠: [improvement-plan-2026-11-05.md](../../0_doc/evaluation/improvement-plan-2026-11-05.md) **P0-3** · [x0081](x0081-control-plane-pop.md)（CAS はスコープ外）· [x006d](x006d-host-take-out-control.md) / [x007b](x007b-scene-despawn.md)

---

## 保証する / しない

| 保証する | 保証しない |
|:---|:---|
| `POST/DELETE …/scene/entries*` は `expected_revision` 必須。不一致・CAS 敗敗は **409** `scene_revision_conflict` | Room 単位グローバル直列化 alone；無限再試行 |
| `UPDATE rooms SET scene=? WHERE room_id=? AND json_extract(scene,'$.revision')=?`（列追加なし） | `scene_revision` 独立カラム；分散 2PC（world↔host↔guest） |
| 競合時も敗者の再試行後、両 mutation が最終 scene に反映可能 | PoP / Token 契約の変更（x0081）；host-token RL（P1） |
| host spawn/despawn が 409 時 GET scene → 有限回再送 | フル UGC エディタ；guest への scene 配布 |

## 設計（要点）

```
host ──expected_revision=N──► world
world ──CAS UPDATE revision=N──► DB
  ├─ num_rows=1 → 200 scene (rev N+1)
  └─ num_rows=0 → 409 scene_revision_conflict
host ──GET scene──► 再試行（上限付き）
```

- DELETE も PoP のため JSON body `{"expected_revision":N}`（空 body 廃止）
- lease の `:conflict`（`host_lease_conflict`）と atom / JSON キーを分離

## Phase

| Phase | 内容 | 状態 |
|:---|:---|:---|
| 0 | Slice + README + architecture 下書き | done |
| 1 | `Rooms` CAS + controller 契約 | done |
| 2 | world 競合テスト + 既存追随 | done |
| 3 | host 409 再試行配線 | done |
| 4 | 保証表・backlog・done | done |

## 完了条件

1. `expected_revision` 無し → 422；不一致 → 409 `scene_revision_conflict`
2. append/delete が revision WHERE 付き CAS でのみ成功
3. 同時 append+delete 統合テストで欠落なし（再試行後に両反映）
4. host が scene conflict のみ有限再試行（lease / duplicate は即 Err）
5. `mix test`（world）緑；保証表・Slice・architecture 一致

## やらない（本 Slice）

- `scene_revision` 列マイグレーション
- PoP / session 制御面の変更（x0081）
- ~~relay-agent CSR-only（P0-4）~~ → [x0083](x0083-relay-agent-csr-only.md)
- host-token 操作別 rate limit（P1）
- 新規 Hex / Cargo 依存

## 関連

- [x006d-host-take-out-control.md](x006d-host-take-out-control.md)
- [x007b-scene-despawn.md](x007b-scene-despawn.md)
- [x0081-control-plane-pop.md](x0081-control-plane-pop.md)
- [improvement-plan-2026-11-05.md](../../0_doc/evaluation/improvement-plan-2026-11-05.md)

← [目次](../README.md)
