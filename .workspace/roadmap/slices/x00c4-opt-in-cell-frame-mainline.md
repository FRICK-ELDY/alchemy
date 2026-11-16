# x00c4 — Spatial Interest: opt-in cell 本線（Room frame 非購読）

> **ID:** `x00c4` · **slug:** `opt-in-cell-frame-mainline` · **status:** `done` · **area:** `[protocol, client, docs]` · **depends_on:** `[x00c3]`  
> **優先度:** [spatial_interest.md](../spatial_interest.md) 実装第七波（P3-7）  
> **後続:** [x00c5](x00c5-default-cell-frame-mainline.md) 既定 cell 切替 · avatar LOD · G1/G2 実測 · host dual-publish 縮小（次空き `x00c6`〜）

目標: 既定は Room `…/frame` のみのまま、`ALCHEMY_CELL_FRAME=1` 時は **Room `…/frame` を購読せず** 近傍 cell を tick 本線にする。gap→resync は cell 経路が **Room `…/snapshot`** を起動する。host dual-publish・Room キー・既定購読は維持（受信側の段階縮小）。

根拠: [spatial_interest.md](../spatial_interest.md) §4 · [x00c3](x00c3-multi-cell-same-tick-merge.md) · x0024

---

## 着手ゲート（満たしていること）

| 条件 | 状態 |
|:---|:---|
| x00c3 多 cell 同一 tick 合流（Room 無し境界横断） | **done** |
| x00c1 自位置追従 + x00c2 セル退出 remove | **done** |
| Room AEAD / wire_cap / x0024 を壊さない | 本スライスで **契約維持** |
| 独自トランスポート禁止 | 維持（Zenoh のみ） |

破ってはならないこと: Room `…/frame` 廃止、client 既定購読を cell のみへ切替、Room dual-publish 停止、snapshot セル分割、受信者別 `V_ack`、検証スキップ、`frame/**` ACL 化、avatar LOD / CCU 天井拡大の宣言。

---

## 保証する / しない（本スライス）

| 保証する | 保証しない |
|:---|:---|
| 既定（env 未設定）は Room `…/frame` のみ | 既定 cell ON / `ALCHEMY_ROOM_FRAME` |
| `ALCHEMY_CELL_FRAME=1` で Room frame **非購読**・Moore cell のみ | cell put 到達保証 |
| opt-in 時、cell `NeedResync` が Room `…/snapshot` を get | cell 別 snapshot |
| host dual-publish（Room fail-closed + cell soft-put）維持 | dual-publish 停止・フルメッシュ廃止 |
| x00c1 follow / x00c3 合流は opt-in 本線でも維持 | avatar LOD / CCU 天井拡大 |
| Room same-tick override API・単体は退行防止で残す | opt-in 実行時の Room 併読安全網 |
| — | host cell soft-put 失敗時の欠落を Room 購読で補完すること（opt-in は Room frame 非購読） |
| — | Moore / AOI 外へ出た・遠方 despawn の client 側 prune（初期 snapshot 後の遠方ゴースト除去は非保証・後続） |

---

## 実装メモ

* Env: `ALCHEMY_CELL_FRAME`（`CELL_FRAME_SUB_ENV`）。既定 false。truthy 同型（`1` / `true` / `yes`）
* `ensure_room_bus`: `frame_subscribe_mode(cell_frame_sub_enabled())` → `FrameSubscribeMode::{Room, CellMainline}` で spawn を排他切替（ON=cell のみ / OFF=Room のみ）
* cell 経路: `gap_resync=true`（Room snapshot get；セル get なし）
* ホットパス: opt-in 中は Room 全文の二重 AEAD/decode をしない（帯域・CPU の受信側縮小）
* host は引き続き Room + cell dual-publish（本スライスでは publish 側を触らない）

---

## Phase

| Phase | 内容 | 状態 |
|:---|:---|:---|
| 0 | Slice + roadmap / spatial_interest 目次 | **done** |
| 1 | client 購読排他 + cell gap→Room snapshot + 単体 | **done** |
| 2 | peer-delivery / architecture 追随 + `done` | **done** |

---

## 完了条件

1. 既定（env 未設定）では Room `…/frame` のみ購読（退行なし）
2. `ALCHEMY_CELL_FRAME=1` で Room frame を購読せず cell Moore のみ購読する
3. opt-in 時、cell tick gap で Room `…/snapshot` を get する
4. host dual-publish・既定 cell 切替・Room 廃止は非保証のまま文書化
5. `cargo test -p alchemy-protocol` / client `bus::frame` / client `room::tests`（`frame_subscribe_mode_*`）Green
6. peer-delivery / architecture / spatial_interest の保証表がコードと一致

---

## 起動・検証メモ

```powershell
$env:ALCHEMY_CELL_FRAME = "1"
# 通常どおり client + host；ログに cell mainline（Room frame not subscribed）が出ること
cargo test -p alchemy-protocol
# from client/
cargo test -p client --bin client -- bus::frame
cargo test -p client --bin client -- room::tests frame_subscribe_mode
```

推奨ブランチ: `feat/x00c4-opt-in-cell-frame-mainline`
