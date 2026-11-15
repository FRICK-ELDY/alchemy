# x00be — Spatial Interest: cell frame 契約 + dual-publish 基盤

> **ID:** `x00be` · **slug:** `spatial-interest-cell-frame-foundation` · **status:** `done` · **area:** `[protocol, relay, host, docs]` · **depends_on:** `[x00b1]`  
> **優先度:** [spatial_interest.md](../spatial_interest.md) 実装第一波（P3-7）  
> **後続:** [x00bf](x00bf-client-near-cell-frame-sub.md) client 近傍セル opt-in 併読（**done**）· [x00c0](x00c0-host-cell-dirty-filter.md) host セル別 dirty（**done**）· avatar LOD · G1/G2 実測（次空き `x00c1`〜）

目標: 規模本線の **セル修飾 frame キー契約（方向 A）** を固定し、WAN ACL 明示許可と host の dual-publish（Room `…/frame` 維持＋`…/frame/cell/0_0_0`）まで配線する。client 既定購読・LOD・フルメッシュ廃止は後続。

根拠: [spatial_interest.md](../spatial_interest.md) §3–§4 · [improvement-plan-2026-11-12.md](../../0_doc/evaluation/improvement-plan-2026-11-12.md) P3-7 · x0024 / x0067 / x0069

---

## 着手ゲート（満たしていること）

| 条件 | 状態 |
|:---|:---|
| MVCC Phase 3 dirty 本線 x00b1 | **done** |
| Room AEAD / wire_cap / x0024 を壊さない | 本スライスで **契約維持** |
| 独自トランスポート禁止 | 維持（Zenoh のみ） |

破ってはならないこと: `…/frame` → `…/frame/**` 一括 ACL、Room `…/frame` の廃止、client 既定購読切替、snapshot セル分割、受信者別 `V_ack`、検証スキップ、guest sim put 許可、avatar LOD / CCU 天井拡大の宣言。

---

## 保証する / しない（本スライス）

| 保証する | 保証しない |
|:---|:---|
| キー式 `…/frame/cell/{cx}_{cy}_{cz}`（`format_cell_id_segment` / `frame_cell_key`） | セル別 dirty フィルタ・帯域削減 |
| Room `…/frame` を tick 正本として残す | client が cell キーを既定購読すること |
| host が Room frame 成功後に `…/frame/cell/0_0_0` へ同一密封バイトを dual-publish | 複数セル publish・遠方セル |
| WAN ACL に `…/frame/cell/*` を **明示**（host-put-sim / host-egress-declare / guest-sub-sim / guest-egress-sim） | `frame/**` ワイルドカード |
| cell put 失敗は soft-fail（Room 経路維持）+ 1s physics ログの `cell_ok` / `cell_fail` 窓計測 | cell put の到達保証 |
| 欠落再同期は現行 Room `…/snapshot` | cell 別 snapshot |
| AEAD AAD・PROTOCOL_PIN 据え置き | キー式を AAD に束縛 |

---

## 実装メモ

* `protocol/src/lib.rs` — `format_cell_id_segment` / `parse_cell_id_segment` / `frame_cell_key` / `FRAME_CELL_KEY_LOCAL_ORIGIN`
* セル索引は shared-sim `CellId`（`CELL_SIZE_M=64` center origin）と同系；protocol は **i32 三段の整形のみ**（`shared-sim` 非依存）
* 負座標は `-1_2_0` 形式；`-1_-2_0` は符号付き整数の逐次 parse（単純 `_` split 禁止）
* ACL: `alchemy/dev/room/*/*/frame/cell/*`（inventory と同様の明示列挙）
* host（x00be 時点）: `NetworkWorkerConfig.frame_cell_pub`；Room 成功後に origin `0_0_0` soft dual-put
* **現行（x00c0）**: 単一 `frame_cell_pub` は廃止。`session` + LRU cell publisher キャッシュで非空 `CellId` へセル別密封 soft-put（詳細は [x00c0](x00c0-host-cell-dirty-filter.md)）

---

## Phase

| Phase | 内容 | 状態 |
|:---|:---|:---|
| 0 | Slice + roadmap / spatial_interest キー固定 | **done** |
| 1 | protocol ヘルパ + peer-delivery + 単体否定系 | **done** |
| 2 | WAN ACL 明示行 | **done** |
| 3 | host dual-publish + 回帰 | **done** |
| 4 | architecture 追随 + roadmap `done` | **done** |

---

## 完了条件

1. `frame_cell_key("local", LOCAL_WIRE_CAP, 0, 0, 0)` が `…/frame/cell/0_0_0` になる
2. `parse_cell_id_segment` が `-1_-2_0` を往復し、`/` 混入・不正を拒否する
3. WAN ACL（`config.wan.json5` / `config.wan.e2e.json5`）に cell 明示行があり、`frame/**` 化していない
4. host が Room frame に加え `frame/cell/0_0_0` へ dual-publish する
5. 既存 client は `…/frame` のみで従来どおり（購読変更なし）
6. `cargo test -p alchemy-protocol` / host-server 関連 Green
7. peer-delivery / spatial_interest / architecture の保証表がコードと一致

---

## 起動・検証メモ

```powershell
cargo test -p alchemy-protocol
# from client/
cargo test -p host-server --bin host-server
```

推奨ブランチ: `feat/x00be-spatial-interest-cell-frame-foundation`
