# x00c5 — Spatial Interest: 既定 cell 本線（`ALCHEMY_ROOM_FRAME` オプトアウト）

> **ID:** `x00c5` · **slug:** `default-cell-frame-mainline` · **status:** `done` · **area:** `[protocol, client, docs]` · **depends_on:** `[x00c4]`  
> **優先度:** [spatial_interest.md](../spatial_interest.md) 実装第八波（P3-7）  
> **後続:** avatar LOD · G1/G2 実測 · host dual-publish 縮小 · Moore 外 prune（次空き `x00c6`〜）

目標: client の frame **既定購読**を Moore cell 本線にする（x00c4 の `ALCHEMY_CELL_FRAME=1` 相当）。レガシー Room `…/frame` 全文は **`ALCHEMY_ROOM_FRAME=1`** でオプトアウト購読。gap→resync は引き続き Room `…/snapshot`。host dual-publish・Room キーは維持（受信側の段階縮小の完了）。

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
| 既定（env 未設定）は Moore cell 本線（Room frame **非購読**） | host dual-publish 停止・フルメッシュ廃止 |
| `ALCHEMY_ROOM_FRAME=1` で Room `…/frame` のみ（XOR） | Room `…/frame` キー廃止 |
| cell `NeedResync` が Room `…/snapshot` を get | cell 別 snapshot / cell put 到達保証 |
| host dual-publish（Room fail-closed + cell soft-put）維持 | avatar LOD / CCU 天井拡大 |
| x00c1 follow / x00c3 合流は既定本線でも維持 | Moore / AOI 外の client 側 prune |
| — | `ALCHEMY_CELL_FRAME` による既定制御（legacy・既定決定に使わない） |
| — | host cell soft-put 失敗時の欠落を Room 購読で補完すること（既定は Room frame 非購読） |

---

## 実装メモ

* Env: `ALCHEMY_ROOM_FRAME`（`ROOM_FRAME_SUB_ENV`）。既定 false（cell 本線）。truthy 同型（`1` / `true` / `yes`）で Room のみ
* 優先順位: ROOM truthy → Room；それ以外 → CellMainline
* `ALCHEMY_CELL_FRAME`（`CELL_FRAME_SUB_ENV`）: 互換 API 残置・既定決定には使わない（`#[deprecated]`；設定時は `ensure_room_bus` が warn）
* `ensure_room_bus`: `frame_subscribe_mode(room_frame_sub_enabled())` → 排他 spawn
* cell 経路: `gap_resync=true`（Room snapshot get；セル get なし）
* host は引き続き Room + cell dual-publish（本スライスでは publish 側を触らない）

---

## Phase

| Phase | 内容 | 状態 |
|:---|:---|:---|
| 0 | Slice + roadmap / spatial_interest 目次 | **done** |
| 1 | protocol ROOM env + client 既定反転 + 単体 | **done** |
| 2 | peer-delivery / architecture 追随 + `done` | **done** |

---

## 完了条件

1. 既定（env 未設定）では Room `…/frame` を購読せず cell Moore のみ購読する
2. `ALCHEMY_ROOM_FRAME=1` で Room `…/frame` のみ購読する（cell 非購読）
3. cell 本線時、tick gap で Room `…/snapshot` を get する
4. host dual-publish・Room 廃止・LOD は非保証のまま文書化
5. `cargo test -p alchemy-protocol` / client `bus::frame` / client `room::tests`（`frame_subscribe_mode_*`）Green
6. peer-delivery / architecture / spatial_interest の保証表がコードと一致

---

## 起動・検証メモ

```powershell
# 既定: cell mainline（Room frame not subscribed）
# 通常どおり client + host；ログに cell Moore mainline が出ること

$env:ALCHEMY_ROOM_FRAME = "1"
# Room …/frame のみ購読に戻ること

cargo test -p alchemy-protocol
# from client/
cargo test -p client --bin client -- bus::frame
cargo test -p client --bin client -- room::tests frame_subscribe_mode
```

推奨ブランチ: `feat/x00c5-default-cell-frame-mainline`
