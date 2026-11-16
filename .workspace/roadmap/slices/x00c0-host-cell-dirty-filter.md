# x00c0 — Spatial Interest: host セル別 dirty フィルタ（複数セル publish）

> **ID:** `x00c0` · **slug:** `host-cell-dirty-filter` · **status:** `done` · **area:** `[host, client, protocol, docs]` · **depends_on:** `[x00bf]`  
> **優先度:** [spatial_interest.md](../spatial_interest.md) 実装第三波（P3-7）  
> **後続:** [x00c1](x00c1-client-cell-follow-resub.md) 自位置追従（**done**）· Room `…/frame` 依存の段階縮小 · セル退出 remove · avatar LOD · G1/G2 実測（次空き `x00c2`〜）

目標: host が dirty upsert を `CellId` でバケットし、非空セルへ `…/frame/cell/{cx}_{cy}_{cz}` を**セル別密封**で soft-put する。Room `…/frame` は tick 正本として全文維持。既定購読切替・フルメッシュ廃止は後続。

根拠: [spatial_interest.md](../spatial_interest.md) §4–§5 · [x00be](x00be-spatial-interest-cell-frame-foundation.md) · [x00bf](x00bf-client-near-cell-frame-sub.md)

---

## 着手ゲート（満たしていること）

| 条件 | 状態 |
|:---|:---|
| x00be cell キー + ACL + dual-publish 基盤 | **done** |
| x00bf client opt-in 近傍併読 | **done** |
| Room AEAD / wire_cap / x0024 を壊さない | 本スライスで **契約維持** |
| 独自トランスポート禁止 | 維持（Zenoh のみ） |

破ってはならないこと: Room `…/frame` 廃止、client 既定購読を cell のみへ切替、snapshot セル分割、受信者別 `V_ack`、検証スキップ、`frame/**` ACL 化、avatar LOD / CCU 天井拡大の宣言。

---

## 保証する / しない（本スライス）

| 保証する | 保証しない |
|:---|:---|
| Room `…/frame` は従来どおり全文 dirty（fail-closed） | Room 購読の廃止・既定 cell 切替 |
| dirty upsert を `CellId::from_world_f64`（`CELL_SIZE_M=64` center origin）でバケット | avatar LOD / CCU 天井拡大 |
| 非空バケットを `…/frame/cell/{cx}_{cy}_{cz}` へ **セル別密封**で soft-put | cell put 到達保証 |
| 空バケットは put しない | cell 別 snapshot |
| 同一 `net_tick` を Room / 各 cell にスタンプ | — |
| 併読時、同一 tick で Room が部分 cell 適用を上書き（`decide_room_frame_tick`） | cell-only 既定化後の多 cell 同一 tick 合流 |
| removes は Room 全文。cell 側には同梱しない | セル退出の即時 remove 保証 |
| cell seal/put は soft-fail（Room 成功後に process を殺さない） | cell put 到達保証 |
| cell publisher キャッシュ上限 64 の **LRU**（同時アクティブ想定；declare 成功後に evict） | 無制限 publisher；cell 経路の Entity zero-clone |

---

## 実装メモ

* host `network_worker`: `bucket_upsert_indices_by_cell` → セルごと sign / soft-seal / soft-put
* `NetworkWorkerConfig`: `session` + `wire_cap` + LRU publisher キャッシュ（64）
* セル判定は `alchemy_shared_sim::mvcc::CellId`（protocol に `CELL_SIZE` を複製しない）
* 統計: `cell_ok` / `cell_fail` + `cell_skip`（LRU eviction 回数）
* client: `WorldView.last_frame_from_room` + Room 同 tick 上書き（x00bf 併読維持）

---

## Phase

| Phase | 内容 | 状態 |
|:---|:---|:---|
| 0 | Slice + roadmap / spatial_interest 目次 | **done** |
| 1 | host バケット + multi soft-put + 単体 | **done** |
| 2 | peer-delivery / architecture 追随 + `done` | **done** |

---

## 完了条件

1. Room `…/frame` は全文 dirty のまま（退行なし）
2. 非空セルへセル別密封の soft-put が走る（同一密封バイト dual-publish をやめる）
3. `CellId` 境界（31.9 → `0_0_0`、32.0 → `1_0_0`）の単体 Green
4. 空バケットは put しない
5. publisher キャッシュ上限超過は **declare 成功後**に LRU eviction（Room 経路維持；失敗時は容量を減らさない）
6. 併読時、同一 tick の部分 cell 適用を Room が上書きできる
7. `cargo test -p host-server --bin host-server` / client `bus::frame` / protocol tick 単体 Green
8. peer-delivery / architecture / spatial_interest の保証表がコードと一致

---

## 起動・検証メモ

```powershell
cargo test -p alchemy-protocol
# from client/
cargo test -p host-server --bin host-server
```

推奨ブランチ: `feat/x00c0-host-cell-dirty-filter`
