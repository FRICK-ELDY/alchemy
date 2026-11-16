# x00c2 — Spatial Interest: セル退出 remove（host）

> **ID:** `x00c2` · **slug:** `cell-exit-remove` · **status:** `done` · **area:** `[host, client, docs]` · **depends_on:** `[x00c1]`  
> **優先度:** [spatial_interest.md](../spatial_interest.md) 実装第五波（P3-7）  
> **後続:** [x00c3](x00c3-multi-cell-same-tick-merge.md) 多 cell 同一 tick 合流 · Room 既定切替 · avatar LOD · G1/G2 実測（次空き `x00c4`〜）

目標: host が前回配信セルを追跡し、セル境界横断・デスポーン時に **旧セル** `…/frame/cell/…` へ removes を soft-put する。Room `…/frame` 全文 removes と併読契約は維持。client 既定購読切替・多 cell 同一 tick 合流は後続。

根拠: [spatial_interest.md](../spatial_interest.md) §4–§5 · [x00c0](x00c0-host-cell-dirty-filter.md) · [x00c1](x00c1-client-cell-follow-resub.md)

---

## 着手ゲート（満たしていること）

| 条件 | 状態 |
|:---|:---|
| x00c0 host セル別 dirty | **done** |
| x00c1 client 自位置追従 resub | **done** |
| Room AEAD / wire_cap / x0024 を壊さない | 本スライスで **契約維持** |
| 独自トランスポート禁止 | 維持（Zenoh のみ） |

破ってはならないこと: Room `…/frame` 廃止、client 既定購読を cell のみへ切替、snapshot セル分割、受信者別 `V_ack`、検証スキップ、`frame/**` ACL 化、avatar LOD / CCU 天井拡大の宣言、多 cell 同一 tick 合流の実装。

---

## 保証する / しない（本スライス）

| 保証する | 保証しない |
|:---|:---|
| Room `…/frame` は全文 dirty + 全文 removes（fail-closed） | Room 購読の廃止・既定 cell 切替 |
| `last_wire_cell` で前回配信セルを追跡（Room put 成功後に更新） | cell put 到達保証 |
| セル境界横断時、**旧セル**へ remove をセル別密封 soft-put | 中間セルへの連鎖 remove |
| Room `removes` / `removes_only` も last セルへ remove-only soft-put | cell 別 snapshot |
| セル frame は upserts（可空）+ セルスコープ removes；`acked` は空 | cell-only 既定化（多 cell 合流は [x00c3](x00c3-multi-cell-same-tick-merge.md)） |
| 同一 `net_tick` を Room / 各 cell にスタンプ | avatar LOD / CCU 天井拡大 |
| cell seal/put は soft-fail（x00c0 据え置き） | — |
| 併読時の Room 同 tick 上書き（`decide_room_frame_tick`）維持 | ギャップ中の cell 欠落収束 |
| upserts∩removes は **remove 勝ち**（cell upsert 除外・`last_wire_cell` も `last_sent` と同順） | 本スライス時点の多 cell 同一 tick 合流（→ [x00c3](x00c3-multi-cell-same-tick-merge.md)） |

---

## 実装メモ

* host `network_worker`: `WorkerState.last_wire_cell: HashMap<u32, [i32; 3]>`
* 入口: `plan_cell_frame_buckets`（upsert 索引・Room removes 除外 + 旧セル remove）→ `materialize_cell_soft_puts`（upsert セルが対になる remove を所有権移動で消費 → 残り remove-only）→ Room 成功後に soft-put（`acked` 空）
* `put_signed_frame` は上記 materialize 結果をそのまま soft-put（結合回帰は `materialize_cell_soft_puts_*` 単体）
* 初回 upsert は exit remove なし；同セル移動も remove なし
* `apply_last_wire_cell`: upsert 後に remove（remove 勝ち；coalesce 交差対応）
* client `apply_frame` は既存 removes 処理のまま（tick ゲート変更なし；併読は Room 同 tick 上書き）

---

## Phase

| Phase | 内容 | 状態 |
|:---|:---|:---|
| 0 | Slice + roadmap / spatial_interest 目次 | **done** |
| 1 | host `last_wire_cell` + exit remove soft-put + 単体 | **done** |
| 2 | peer-delivery / architecture 追随 + `done` | **done** |

---

## 完了条件

1. セル跨ぎで旧セル topic に remove が載る
2. despawn / `removes_only` でも last セルへ remove-only soft-put
3. Room 全文 removes・fail-closed・cell soft-fail は退行なし
4. 同セル移動では exit remove が出ない
5. catch-up 後に `last_wire_cell` が整合する
6. 多 cell 同一 tick 合流・既定購読切替は非保証のまま文書化
7. `cargo test -p host-server --bin host-server` / client `bus::frame` / protocol 単体 Green
8. peer-delivery / architecture / spatial_interest の保証表がコードと一致

---

## 起動・検証メモ

```powershell
cargo test -p alchemy-protocol
# from client/
cargo test -p host-server --bin host-server
cargo test -p client --bin client -- bus::frame
```

推奨ブランチ: `feat/x00c2-cell-exit-remove`
