# x00bf — Spatial Interest: client 近傍セル購読（opt-in / 併読）

> **ID:** `x00bf` · **slug:** `client-near-cell-frame-sub` · **status:** `done` · **area:** `[protocol, client, docs]` · **depends_on:** `[x00be]`  
> **優先度:** [spatial_interest.md](../spatial_interest.md) 実装第二波（P3-7）  
> **後続:** [x00c0](x00c0-host-cell-dirty-filter.md) host セル別 dirty（**done**）· Room `…/frame` 依存の段階縮小 · 自位置追従の購読切替 · avatar LOD · G1/G2 実測（次空き `x00c1`〜）

目標: x00be の cell frame キーを **client が opt-in で近傍セル購読**できるようにする。既定は Room `…/frame` のみ。opt-in 時は Room を維持したまま近傍 `…/frame/cell/…` を**併読**する（フルメッシュ廃止・帯域削減は後続）。

根拠: [spatial_interest.md](../spatial_interest.md) §4–§5 · [x00be](x00be-spatial-interest-cell-frame-foundation.md) · x0024

---

## 着手ゲート（満たしていること）

| 条件 | 状態 |
|:---|:---|
| x00be cell キー + ACL + host dual-publish | **done** |
| Room AEAD / wire_cap / x0024 を壊さない | 本スライスで **契約維持** |
| 独自トランスポート禁止 | 維持（Zenoh のみ） |

破ってはならないこと: 既定購読を cell のみへ切替、Room `…/frame` 廃止、snapshot セル分割、受信者別 `V_ack`、検証スキップ、`frame/**` ACL 化、avatar LOD / CCU 天井拡大の宣言、セル別 dirty フィルタ。

---

## 保証する / しない（本スライス）

| 保証する | 保証しない |
|:---|:---|
| 既定は Room `…/frame` のみ（x00be と同じ） | 帯域削減・フルメッシュ解除 |
| `ALCHEMY_CELL_FRAME=1` で origin 近傍 Moore（3³=27）の `…/frame/cell/…` を **併読** | 自位置追従の動的 resubscribe |
| 併読フレームは既存 `apply_frame`（tick / AEAD / sig）を共有。同一密封バイトの二重適用は tick で DropStale | cell 経路だけの到達保証 |
| gap→snapshot は **Room 購読スレッドのみ**（cell は apply のみ） | cell 単独での欠落収束 |
| 欠落再同期は現行 Room `…/snapshot` | cell 別 snapshot |
| protocol に Moore 近傍ヘルパ + interest keyexpr（空→`""`、重複セルは dedup） | host 複数セル publish |

---

## 実装メモ

* Env: `ALCHEMY_CELL_FRAME`（`CELL_FRAME_SUB_ENV`）。truthy は `ALCHEMY_HOST_CONTROL` と同型（`1` / `true` / `yes`）
* protocol: `moore_neighborhood_cells` / `frame_cell_interest_keyexpr`（`|` 結合・dedup）
* client `ensure_room_bus`: Room frame 購読のあと、opt-in 時に origin `(0,0,0)` 近傍 keyexpr を追加購読（host 当面 `0_0_0` dual-publish と整合）
* ホットパス: opt-in 中は Room + origin cell の**同一密封バイトを毎 tick 二重に AEAD/sig/decode**する（帯域削減前の検証コスト；後続で条件付き dual-put / cell-only 切替）
* スレッド名: Room=`zenoh-frame`、cell=`zenoh-frame-cell`
* 中心セルを自位置にする追従切替は後続（バス開始時点では位置が未確定）
* **併読と tick ゲート**: Room / cell は同一 `decide_frame_tick`。**同一 tick は先着勝ち**（後着 DropStale）。当面は同一密封バイトなので順序差は無害。セル別 dirty 後も本スライスは「同一 tick の Room 優先」を実装せず、**次 tick 以降の Room（＋ Room のみ gap→snapshot）で収束**する。cell-only 既定化や同一 tick Room 優先は後続スライス

---

## Phase

| Phase | 内容 | 状態 |
|:---|:---|:---|
| 0 | Slice + roadmap 目次 | **done** |
| 1 | protocol 近傍ヘルパ + env flag + 単体 | **done** |
| 2 | client opt-in 併読配線 | **done** |
| 3 | architecture / peer-delivery / spatial_interest 追随 + `done` | **done** |

---

## 完了条件

1. 既定（env 未設定）では Room `…/frame` のみ購読（退行なし）
2. `ALCHEMY_CELL_FRAME=1` で origin Moore 近傍の cell keyexpr を追加購読する
3. Room `…/frame` は opt-in 中も購読維持（併読）
4. `moore_neighborhood_cells(0,0,0)` が 27 セル・中心を含む
5. `cargo test -p alchemy-protocol` Green；client 既存 frame 単体 Green
6. peer-delivery / client.md / spatial_interest の保証表がコードと一致

---

## 起動・検証メモ

```powershell
$env:ALCHEMY_CELL_FRAME = "1"
# 通常どおり client + host；ログに cell frame 併読購読が出ること
cargo test -p alchemy-protocol
# from client/
cargo test -p client --bin client -- bus::frame
```

推奨ブランチ: `feat/x00bf-client-near-cell-frame-sub`
