# x00c3 — Spatial Interest: 多 cell 同一 tick 合流（Room 依存縮小の第一歩）

> **ID:** `x00c3` · **slug:** `multi-cell-same-tick-merge` · **status:** `done` · **area:** `[protocol, client, docs]` · **depends_on:** `[x00c2]`  
> **優先度:** [spatial_interest.md](../spatial_interest.md) 実装第六波（P3-7）  
> **後続:** [x00c4](x00c4-opt-in-cell-frame-mainline.md) opt-in cell 本線（**done**）· 既定 cell 切替 · avatar LOD · G1/G2 実測（次空き `x00c5`〜）

目標: opt-in cell 経路で同一 `net_tick` の複数セル frame を**合流適用**し、境界横断（新セル upsert + 旧セル remove）が **Room 同 tick 上書き無し**でも収束するようにする。**本スライス当時**は Room 併読・dual-publish・既定購読を維持（意味的依存の解除が第一歩）。client 購読の排他（opt-in cell 本線）は後続 [x00c4](x00c4-opt-in-cell-frame-mainline.md)。

根拠: [spatial_interest.md](../spatial_interest.md) §4 · [x00c2](x00c2-cell-exit-remove.md) · x0024

---

## 着手ゲート（満たしていること）

| 条件 | 状態 |
|:---|:---|
| x00c2 host セル退出 remove | **done** |
| x00c0/x00c1 併読 + Room 同 tick 上書き | **done** |
| Room AEAD / wire_cap / x0024 を壊さない | 本スライスで **契約維持** |
| 独自トランスポート禁止 | 維持（Zenoh のみ） |

破ってはならないこと: Room `…/frame` 廃止、client 既定購読を cell のみへ切替、Room dual-publish 停止、Room 受信スキップ、snapshot セル分割、受信者別 `V_ack`、検証スキップ、`frame/**` ACL 化、avatar LOD / CCU 天井拡大の宣言。

---

## 保証する / しない（本スライス）

| 保証する | 保証しない |
|:---|:---|
| cell 経路は `decide_cell_frame_tick`（`tick == prev && !last_from_room` → Apply） | Room 購読の廃止・既定 cell 切替 |
| 同一 tick・複数 cell を合流適用できる | cell put 到達保証 |
| 単一 frame 内 upserts∩removes は **remove 勝ち**（host 契約据え置き） | cell 別 snapshot |
| 同一 tick・跨 cell では先着 upsert 生存 id を guard；後着 **当該 id の remove-only**（同 frame にその id の upsert 無し）は guard 済みをスキップ（**upsert 勝ち**） | ギャップ中の cell 欠落収束 |
| 境界横断は到着順によらず新位置に残る（Room 無し） | avatar LOD / CCU 天井拡大 |
| Room 併読 + `decide_room_frame_tick` 同 tick 上書きは安全網として維持 | Room 受信スキップ / dual-publish 停止 |
| tick 前進 / Room・snapshot 適用で guard クリア | — |

---

## 実装メモ

* protocol: `decide_cell_frame_tick`（`decide_room_frame_tick` と同型；内部 `decide_partial_frame_tick` 共有）
* client `WorldView`: `cell_tick_upsert_guard: HashSet<u32>`
* `apply_delta_entities`: cell は新ゲート；同一 tick 合流時は **remove-only** が guard 済み id をスキップ（同一 frame 交差は常に remove 勝ち）；frame 適用後に生存 upsert id を guard へ；Room/snapshot/tick 前進で clear
* バッファや追加スレッドは作らない（サンプル到着ごとに合流）
* 既存 `room_overrides_same_tick_after_partial_cell` は退行防止で残す

---

## Phase

| Phase | 内容 | 状態 |
|:---|:---|:---|
| 0 | Slice + roadmap / spatial_interest 目次 | **done** |
| 1 | protocol ゲート + client 合流 + 単体 | **done** |
| 2 | peer-delivery / architecture 追随 + `done` | **done** |

---

## 完了条件

1. 同一 tick・新セル upsert → 旧セル remove が Room 無しで収束（entity 新位置）
2. 同一 tick・旧セル remove → 新セル upsert も Room 無しで収束
3. 単一 frame 交差は remove 勝ちのまま
4. Room 同 tick 上書き・Room 後 cell DropStale は退行なし
5. 既定購読切替・Room 廃止は非保証のまま文書化
6. `cargo test -p alchemy-protocol` / client `bus::frame` Green
7. peer-delivery / architecture / spatial_interest の保証表がコードと一致

---

## 起動・検証メモ

```powershell
cargo test -p alchemy-protocol
# from client/
cargo test -p client --bin client -- bus::frame
```

推奨ブランチ: `feat/x00c3-multi-cell-same-tick-merge`
