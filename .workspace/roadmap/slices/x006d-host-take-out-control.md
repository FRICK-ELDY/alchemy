# x006d — host take-out（制御面）

> **ID:** `x006d` · **slug:** `host-take-out-control` · **status:** `done` · **area:** `[assets, world, host, client, docs]` · **depends_on:** `[x005f, x006c]`  
> **優先度:** backlog P0（inventory API 薄皮の後続）

目標: host role の Desktop Inventory から **単一 blob（1 blake3 mesh）** を部屋に spawn する。経路は Intent ではなく **host 向け低頻度制御面**（Zenoh `…/inventory/spawn`）。assets は Drive HTTP で定義を返し、host は blake3 / SceneEntry だけを実行する。canonical seal のため world が scene を追記する。

根拠: [backlog-priority-2026-11-04.md](../backlog-priority-2026-11-04.md) · x005f（assets resolve）· 会話合意（take-out のみ / 制御面 B）

## 保証する / しない

| 保証する | 保証しない |
|:---|:---|
| assets: `GET /drive/nodes` + `POST /drive/take_out` が owner + Grant 検証済み blake3 を返す | put-back / Prefab 多子展開 / `repo@rev` |
| Desktop Inventory 最小 UI（一覧 + 取り出す）。単一 blob ノードのみ | XR Inventory・Drive フォルダ UX 完成 |
| host role のみが Zenoh inventory spawn で要求可能。host は **Drive `node_id` を受け取らない** | guest take-out；Intent / PROTOCOL_PIN 拡張 |
| world: host が `POST …/scene/entries` で mesh 追記・revision bump（canonical seal 入力） | フル UGC エディタ |
| spawn 成功時 Rapier + shared scene 更新 → **再 seal**。Rapier 失敗時は部分物理 insert なし | `extra_blake3`；PlaceRead TTL UI |
| 再 seal 後の scene hash で guest PlaceRead が新 mesh をカバー可能 | 同時多発 spawn の公平スケジューリング |

## 設計（要点）

```
Client UI ─Bearer─► assets GET /drive/nodes · POST /drive/take_out
Client ─Room Token + blake3─► Zenoh …/inventory/spawn
host ─verify host role─► world POST …/scene/entries
host ─► Rapier spawn_mesh_entry + force_reseal
```

- 定義 = assets Drive；実行 = host（client は解決・要求）
- 未認証 `host_control` shutdown トピックは使わない
- WAN ACL: guest put + host sub on `inventory/spawn`（app が role 検証）

## Phase

| Phase | 内容 | 状態 |
|:---|:---|:---|
| 0 | Slice + 目次 / backlog | done |
| 1 | assets Drive HTTP + テスト | done |
| 2 | world scene append + host 動的 spawn + Zenoh 制御面 + 再 seal | done |
| 3 | client Inventory UI + spawn 送信 | done |
| 4 | architecture 注記 + 回帰 | done |

## 完了条件

1. owner が Drive blob ノードを take_out HTTP で解決できる；他 owner / 無 Grant → 4xx
2. host role Desktop Inventory から 1 blake3 を部屋に spawn（Manifest エントリ増）
3. guest / 非 host の spawn 要求は拒否
4. Rapier spawn 失敗時に半端な物理 insert が残らない
5. 保証表とコード一致；assets / world 関連テスト緑

## やらない（本 Slice）

- put-back UI / Intent
- Prefab Tree 展開・ItemRef `repo@rev`
- `extra_blake3`
- guest 发起の take-out
- Slot undo UI
- `goal.md` 更新

← [目次](../README.md)
