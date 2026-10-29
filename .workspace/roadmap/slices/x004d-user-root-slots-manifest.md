# x004d — USER_ROOT_SLOTS を Manifest へ

> **ID:** `x004d` · **slug:** `user-root-slots-manifest` · **status:** `done` · **area:** `[protocol, host, world, docs]` · **depends_on:** `[x0030, x003b]`  
> **優先度:** **P3-10**

目標: 参加者上限を protocol 定数ではなく **world の Manifest entries** から取る。protocol は user_root **ID 帯の天井**だけを持つ。

根拠: [improvement-plan-2026-10-28.md](../../0_doc/evaluation/improvement-plan-2026-10-28.md) **P3-10**

## 保証する / しない

| 保証する | 保証しない |
|:---|:---|
| protocol: `USER_ROOT_ENTITY_FIRST` + `USER_ROOT_SLOTS_MAX`（帯）+ `user_root_slots_from_entries` | 5 人超の動的スケール / 帯拡大の本番チューニング |
| 部屋の Presence 容量 = Manifest の連続 user_root entries（`FIRST..FIRST+n-1`、`1..=MAX`） | UGC で slots を実行時変更（boot 時 scene のみ） |
| host: Rapier / Presence は `[T; MAX]` + `slot_count=n`（`by_slot` は Copy スナップショット） | client UBO 予約を部屋ごとに動的再割当（天井 `MAX` で可） |
| DemoScene の `@user_spawns` 本数 = デモ部屋の容量（現状 4） | Room DB 列としての独立 `max_players` フィールド |

## 完了条件

- `USER_ROOT_SLOTS` 容量定数を廃止し、`USER_ROOT_SLOTS_MAX`（帯）+ Manifest 由来 `n` に置換
- host が entries 数と異なる固定 4 で Presence を張らない
- architecture/protocol.md・本スライスとコード一致
- pin 据え置き（ワイヤ schema 不変；容量は既に entries で表現）

← [目次](../README.md)
