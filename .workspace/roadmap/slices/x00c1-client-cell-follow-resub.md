# x00c1 — Spatial Interest: 自位置追従の cell 購読切替

> **ID:** `x00c1` · **slug:** `client-cell-follow-resub` · **status:** `done` · **area:** `[client, docs]` · **depends_on:** `[x00c0]`  
> **優先度:** [spatial_interest.md](../spatial_interest.md) 実装第四波（P3-7）  
> **後続:** Room `…/frame` 依存の段階縮小 · セル退出 remove · avatar LOD · G1/G2 実測（次空き `x00c2`〜）

目標: `ALCHEMY_CELL_FRAME=1` 時の cell frame 購読中心を固定 origin から自位置 `CellId` 追従へ切り替え、中心セル変化時のみ CellInterest を再購読する。Room `…/frame` 併読・gap→snapshot は維持。

根拠: [spatial_interest.md](../spatial_interest.md) §4–§5 · [x00bf](x00bf-client-near-cell-frame-sub.md) · [x00c0](x00c0-host-cell-dirty-filter.md)

---

## 着手ゲート（満たしていること）

| 条件 | 状態 |
|:---|:---|
| x00bf client opt-in 近傍併読 | **done** |
| x00c0 host セル別 dirty | **done** |
| Room AEAD / wire_cap / x0024 を壊さない | 本スライスで **契約維持** |
| 独自トランスポート禁止 | 維持（Zenoh のみ） |

破ってはならないこと: Room `…/frame` 廃止、client 既定購読を cell のみへ切替、snapshot セル分割、受信者別 `V_ack`、検証スキップ、`frame/**` ACL 化、avatar LOD / CCU 天井拡大の宣言、セル退出の即時 remove。

---

## 保証する / しない（本スライス）

| 保証する | 保証しない |
|:---|:---|
| 既定（env 未設定）は Room `…/frame` のみ | 帯域削減・フルメッシュ解除 |
| `ALCHEMY_CELL_FRAME=1` で自位置中心 Moore（3³=27）を Room と**併読** | cell put 到達・cell-only 既定 |
| Presence 割当後、中心 `CellId` 変化時に **CellInterest だけ** keyexpr 再購読 | Room スレッドの再起動 |
| 中心評価は **≤250ms 期限**（cell サンプル活性時も飢えない） | 評価のサンプル同期・サブ 250ms 精度 |
| 再購読ギャップ中も Room が tick 正本 | ギャップ中の cell 欠落収束 |
| 境界は shared-sim `CELL_HYSTERESIS_M`（2 m）でフラップ抑制 | 物理 `cross_cell` との完全同期 |
| 位置源は `local_user_root` の **wire** 座標（表示予測は使わない） | Presence 未割当中の追従 |
| gap→snapshot は Room のみ（cell は apply のみ） | cell 単独での欠落収束 |

---

## 実装メモ

* 初期購読は origin `(0,0,0)` Moore（バス開始時点では位置未確定；x00bf と同じ）
* `LOCAL_USER_ROOT_UNSET` または wire entity 未到着の間は中心を動かさない
* 中心更新: 現中心 origin からの local が `|local| > 32+2` のとき `CellId::from_world_f64` へスナップ（`EntityDirectory::should_cross_cell` と同型ヒステリシス）
* CellInterest スレッド: `recv_timeout` の待ち時間を次評価期限までにし、**サンプル有無に関わらず ≤250ms ごとに中心評価**（活性エリアの 30Hz cell 配信でタイムアウトが飢えないこと）。変化時は Subscriber Drop + 新 Moore keyexpr で `declare_subscriber`
* Room スレッドは触らない；適用パス（AEAD / tick / gap ログのみ）は x00bf/x00c0 据え置き
* protocol に `CELL_SIZE` を複製しない（shared-sim `CellId` を client が直接使用）

---

## Phase

| Phase | 内容 | 状態 |
|:---|:---|:---|
| 0 | Slice + roadmap / spatial_interest 目次 | **done** |
| 1 | client 追従 resub + 単体 | **done** |
| 2 | peer-delivery / architecture 追随 + `done` | **done** |

---

## 完了条件

1. 既定（env 未設定）では Room `…/frame` のみ購読（退行なし）
2. `ALCHEMY_CELL_FRAME=1` で初期 origin Moore を併読し、自位置中心変化時に CellInterest だけ再購読する
3. Room `…/frame` は opt-in 中も購読維持
4. ヒステリシス境界（例: 現中心に対し 31.9 維持 / 34.1 で隣セル）の単体 Green
5. `cargo test -p alchemy-protocol` / client `bus::frame` Green
6. peer-delivery / client.md / spatial_interest の保証表がコードと一致

---

## 起動・検証メモ

```powershell
$env:ALCHEMY_CELL_FRAME = "1"
# 通常どおり client + host；Presence 後に移動すると cell keyexpr が中心変更時だけ差し替わる
cargo test -p alchemy-protocol
# from client/
cargo test -p client --bin client -- bus::frame
```

推奨ブランチ: `feat/x00c1-client-cell-follow-resub`
