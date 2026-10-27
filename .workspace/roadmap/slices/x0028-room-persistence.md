# x0028 — Room 永続（Ash + SQLite）

> **ID:** `x0028` · **slug:** `room-persistence` · **status:** `done` · **area:** `[world, docs]` · **depends_on:** `[x0027]`

目標: `owner_sub` + DemoScene を world 再起動後も同じ答えにする。定義の正本（x0027）を時間方向に延長する（D4 残り）。

正本: [world-server.md](../../0_doc/architecture/world-server.md)

## 設計方針

| 項目 | 内容 |
|:---|:---|
| ORM | Ash 3 + `AshSqlite.DataLayer`（Stage 1） |
| 切替 | 属性は `:string` / `:map` / timestamps のみ。運営規模拡大時は `AshPostgres` に Repo/DataLayer を差し替え可能 |
| API | 既存 `World.Rooms` facade（create / owner / scene）を維持。GenServer Map を廃止 |
| scene | DemoScene map を JSON カラムとして保存（entries 正規化は後続） |

## 保証する

- world 再起動後も同一 `room_id` に対し `owner_sub` と `scene` が復元される
- `POST /rooms` の `already_exists` / owner-only host Token は従来どおり
- ローカル既定は SQLite ファイル（`WORLD_DATABASE_PATH` で上書き可）

## 保証しない

- Presence / Rapier pose の永続
- UGC scene 編集 UI
- Friendship / 公開範囲
- マルチ world ノード強整合
- host の scene 再 fetch（起動時一度のまま）

## 成功条件

1. `mix test` で Rooms 永続テスト緑  
2. compose 再起動後も create 済み部屋の Token / scene が使える  
3. architecture / roadmap が「揮発」から「SQLite 永続」に更新  

← [目次](../README.md)
