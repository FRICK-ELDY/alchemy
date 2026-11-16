# x00c5 — Spatial Interest: 既定 cell 本線（`ALCHEMY_ROOM_FRAME` オプトアウト）

> **ID:** `x00c5` · **slug:** `default-cell-frame-mainline` · **status:** `done` · **area:** `[protocol, client, docs]` · **depends_on:** `[x00c4]`  
> **優先度:** [spatial_interest.md](../spatial_interest.md) 実装第八波（P3-7）  
> **後続:** [x00c6](x00c6-spatial-interest-vr-delivery-contract.md) VR delivery 契約（Moore per-key · Interest enter · 自アバター応答）· その後 avatar LOD · G1/G2 · dual-publish 縮小

目標: client の frame **既定購読**を cell 本線にする（Room `…/frame` 非購読）。レガシー Room 全文は **`ALCHEMY_ROOM_FRAME=1`** でオプトアウト。gap→resync は Room `…/snapshot`。host dual-publish 維持。

**暫定購読 key**: `…/frame/cell/*`（[`frame_cell_wildcard_key`](../../../protocol/src/lib.rs)）。x00bf–x00c4 の Moore `|` union は local peer で declare OK・配送ゼロを観測したため本スライスでは使わない（再現メモは下）。Moore 再配線は後続。

根拠: [spatial_interest.md](../spatial_interest.md) §4 · [x00c4](x00c4-opt-in-cell-frame-mainline.md) · x0024

---

## 着手ゲート（満たしていること）

| 条件 | 状態 |
|:---|:---|
| x00c4 opt-in cell 本線（Room frame 非購読） | **done** |
| x00c1 自位置追従 + x00c3 多 cell 合流 | **done** |
| Room AEAD / wire_cap / x0024 を壊さない | 本スライスで **契約維持** |
| 独自トランスポート禁止 | 維持（Zenoh のみ） |

破ってはならないこと: Room `…/frame` 廃止、Room dual-publish 停止、snapshot セル分割、受信者別 `V_ack`、検証スキップ、`frame/**` ACL 化、avatar LOD / CCU 天井拡大の宣言。

---

## 保証する / しない（本スライス）

| 保証する | 保証しない |
|:---|:---|
| 既定（env 未設定）は cell 本線（Room frame **非購読**；購読=`…/frame/cell/*`） | host dual-publish 停止・フルメッシュ廃止 |
| `ALCHEMY_ROOM_FRAME=1` で Room `…/frame` のみ（XOR） | Room `…/frame` キー廃止 |
| cell `NeedResync` が Room `…/snapshot` を get | cell 別 snapshot / cell put 到達保証 |
| host dual-publish（Room fail-closed + cell soft-put）維持 | avatar LOD / CCU 天井拡大 |
| x00c3 合流；wire center ヒステリシス**追跡**（再購読なし） | Moore `|` union 配送・受信側 interest 縮小（暫定 `cell/*`） |
| — | `ALCHEMY_CELL_FRAME` による既定制御（legacy・既定決定に使わない） |
| — | host cell soft-put 失敗時の欠落を Room 購読で補完；Moore / AOI 外 prune |

---

## 実装メモ

* Env: `ALCHEMY_ROOM_FRAME`（`ROOM_FRAME_SUB_ENV`）。既定 false（cell 本線）。truthy 同型（`1` / `true` / `yes`）で Room のみ
* 優先順位: ROOM truthy → Room；それ以外 → CellMainline
* `ALCHEMY_CELL_FRAME`（`CELL_FRAME_SUB_ENV`）: 互換 API 残置・既定決定には使わない（`#[deprecated]`；設定時は `ensure_room_bus` が warn）
* `ensure_room_bus`: `frame_subscribe_mode(room_frame_sub_enabled())` → 排他 spawn
* cell 購読: `frame_cell_wildcard_key` → `…/frame/cell/*`（中心非依存・再購読なし）
* cell 経路: `gap_resync=true`（Room snapshot get；セル get なし）
* host は引き続き Room + cell dual-publish（本スライスでは publish 側を触らない）

### Moore `|` union 配送ゼロ（再現メモ · 戻すときの根拠）

| 項目 | 値 |
|:---|:---|
| 観測日 | 2026-11-16 |
| Zenoh | **1.10.1**（`client/Cargo.lock`） |
| トポロジ | client peer + local zenohd（`ZENOH_CONNECT=tcp/127.0.0.1:7447`） |
| OS / bin | Windows · `client/target/debug/client.exe` |
| 購読 | Moore `|` union（最大 27 · `frame_cell_interest_keyexpr`） |
| 結果 | `declare_subscriber` **OK**；cell soft-put（host `cell_ok≈30/s`）があるのに client pose が join snapshot のまま（Intent は host 到達） |
| 回避 | `…/frame/cell/*`（`frame_cell_wildcard_key`）で配送・適用回復 |

---

## Phase

| Phase | 内容 | 状態 |
|:---|:---|:---|
| 0 | Slice + roadmap / spatial_interest 目次 | **done** |
| 1 | protocol ROOM env + client 既定反転 + 単体 | **done** |
| 2 | peer-delivery / architecture 追随 + `done` | **done** |

---

## 完了条件

1. 既定（env 未設定）では Room `…/frame` を購読せず `…/frame/cell/*`（暫定 wildcard）のみ購読する
2. `ALCHEMY_ROOM_FRAME=1` で Room `…/frame` のみ購読する（cell 非購読）
3. cell 本線時、tick gap で Room `…/snapshot` を get する
4. host dual-publish・Room 廃止・LOD・Moore `|` 配送は非保証のまま文書化
5. `cargo test -p alchemy-protocol` / client `bus::frame` / client `room::tests`（`frame_subscribe_mode_*`）Green
6. peer-delivery / architecture / spatial_interest / trust-boundary の保証表がコードと一致

---

## 起動・検証メモ

```powershell
# 既定: cell/* interim mainline（Room frame not subscribed）
# 通常どおり client + host；ログに interim cell/* wildcard / frame/cell/* が出ること

$env:ALCHEMY_ROOM_FRAME = "1"
# Room …/frame のみ購読に戻ること

cargo test -p alchemy-protocol
# from client/
cargo test -p client --bin client -- bus::frame
cargo test -p client --bin client -- room::tests frame_subscribe_mode
```

推奨ブランチ: `feat/x00c5-default-cell-frame-mainline`
