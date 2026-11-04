# x006e — Inventory put-back UI

> **ID:** `x006e` · **slug:** `inventory-put-back-ui` · **status:** `done` · **area:** `[assets, client, docs]` · **depends_on:** `[x0060, x006d]`  
> **優先度:** backlog P0-次（host take-out の対）

目標: Desktop Inventory から **部屋の単一 mesh（1 blake3）** を Prefab として Drive に戻す。assets は `Drive.save_prefab` を HTTP で公開する。経路は user Bearer → assets のみ（Zenoh / host 制御面は使わない）。元 entity は **部屋に残す**。

根拠: [backlog-priority-2026-11-04.md](../backlog-priority-2026-11-04.md) P0-次 · x0060（save_prefab）· x006d（take-out UI 対称）

## 保証する / しない

| 保証する | 保証しない |
|:---|:---|
| assets: `POST /drive/put_back` が owner Bearer + 検証済み Prefab.Payload で `save_prefab` する | scene からの entity 削除 / reseal；Zenoh put-back 制御面 |
| Desktop Inventory: Manifest 由来の scene mesh 一覧 + **Put back**（単一 child Prefab） | Prefab 多子 UI；XR Inventory；held-grab 必須導線 |
| 保存後も **元 entity は部屋に残る**（コピー保存。方針は本 Slice で固定） | take-out の逆トランザクション；分散 TX |
| put-back 名は非空・255 以内。同一 owner+name+parent は upsert（x0060） | 名前入力 UI の完成（既定名 `putback-e{entity_id}-{blake3[0..8]}`） |
| 保証表・architecture・テストが一致 | `extra_blake3`；Prefab 子 blake3 の Grant 検証（後続）；UI スレッド同期 HTTP（take-out と同型） |

## 設計（要点）

```
Manifest ─► client SharedState scene mesh map (entity_id → blake3)
Client UI ─Bearer─► assets POST /drive/put_back { name, payload, parent_id? }
assets ─► Drive.save_prefab (validate → grant_required upload → prefab upsert)
（部屋 scene は変更しない）
```

- 定義の永続化 = assets Drive；実行側 host は関与しない
- take-out（x006d）は Drive→部屋；put-back は部屋参照→Drive（部屋は不変）
- 選択対象 = Manifest 上の mesh blake3 付きエントリ（user_root / cuboid-only は除外）

## Phase

| Phase | 内容 | 状態 |
|:---|:---|:---|
| 0 | Slice + 目次 / backlog | done |
| 1 | assets: `POST /drive/put_back` + テスト | done |
| 2 | client: scene mesh 保持 + Inventory Put back UI | done |
| 3 | architecture 注記 + 回帰 | done |

## 完了条件

1. owner が単一 child Prefab を `POST /drive/put_back` で保存できる；不正 payload → 4xx
2. Desktop Inventory から scene mesh を選び Put back → Drive に prefab ノードが増える（Refresh で確認可）
3. put-back 後も部屋の該当 entity が残る
4. 保証表とコード一致；assets 関連テスト緑

## やらない（本 Slice）

- world scene DELETE / host despawn / reseal
- Prefab 多子展開・ItemRef `repo@rev` take-out
- Intent 経路
- `goal.md` 更新

← [目次](../README.md)
