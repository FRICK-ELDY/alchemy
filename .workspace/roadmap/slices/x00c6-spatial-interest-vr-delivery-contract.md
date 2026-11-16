# x00c6 — Spatial Interest: VR delivery 契約（Moore per-key · enter 収束 · 自アバター応答）

> **ID:** `x00c6` · **slug:** `spatial-interest-vr-delivery-contract` · **status:** `done` · **area:** `[protocol, client, docs]` · **depends_on:** `[x00c5]`  
> **優先度:** [spatial_interest.md](../spatial_interest.md) 実装第九波（P3-7 · §5 VR delivery contract）  
> **後続:** [x00c7](x00c7-avatar-sync-lod-mid-band.md) Avatar sync LOD · G1/G2 実測 · host dual-publish 縮小（enter 代替が出来てから）

目標: x00c5 暫定 `…/frame/cell/*` を**既定本線から外し**、近傍帯 Interest・Interest enter 収束・自アバター応答を同一契約として実装する。応急（Room オプトアウト固定・gap 定数の場当たり緩和）は本線修正と呼ばない。

根拠: [spatial_interest.md](../spatial_interest.md) §5 · [peer-delivery.md](../../../protocol/peer-delivery.md) · x00ab · x0024

---

## 着手ゲート（満たしていること）

| 条件 | 状態 |
|:---|:---|
| x00c5 既定 cell 本線（`ALCHEMY_ROOM_FRAME` オプトアウト） | **done**（暫定 `cell/*` 含む） |
| x00c1 中心ヒステリシス追跡 · x00c3 多 cell 合流 | **done** |
| x00ab owned prediction（local root / Grab） | **done**（契約昇格は本スライス） |
| Room AEAD / wire_cap / x0024 を壊さない | 本スライスで **契約維持** |
| 独自トランスポート禁止 | 維持（Zenoh のみ） |
| [spatial_interest.md](../spatial_interest.md) §5 正本 | **done** |

破ってはならないこと: Room AEAD / ManifestSeal / 検証スキップ、`frame/**` ACL 化、cell 別 snapshot キー新設、dual-publish 停止の同時実施（enter が Room snapshot 依存）、avatar LOD / CCU 天井拡大の宣言、確実配送 ACK 新設。

---

## 保証する / しない（本スライス）

| 保証する | 保証しない |
|:---|:---|
| 既定 cell 本線は **Moore 3³ per-key ≤27**（ヒステリシス差分 resub） | 既定本線としての `…/frame/cell/*` |
| Moore 集合に cell が入ったとき **Interest enter** → rate-limited Room `…/snapshot` で静止物も含め再可視 | AOI 外に居る間の連続可視・軌跡再生 |
| Interest exit（snapshot 後または明示 prune）で Moore 外ゴーストを残さない | cell 別 snapshot / cell put 到達 |
| NeedResync / snapshot cooldown 中も **MoveIntent publish と prediction tick 継続** | wire 遅れゼロ |
| `ALCHEMY_ROOM_FRAME=1` の Room オプトアウト維持 | host dual-publish 停止・フルメッシュ廃止 |
| x00c3 合流・gap→Room snapshot 維持 | `FRAME_TICK_GAP_RESYNC` 緩和（計測後に別判断） |
| — | Moore `|` union 配送（per-key Green 後の任意最適化） |
| — | avatar LOD 中/遠帯 · CCU 天井拡大 |

---

## 実装波

| 波 | 内容 | 状態 |
|:---|:---|:---|
| **x00c6a** | `cell/*` 既定廃止 → ≤27 per-cell subscriber + ヒステリシス差分 resub | **done** |
| **x00c6b** | AOI enter → rate-limited Room snapshot；exit prune | **done** |
| **x00c6c** | 予測 / Intent を resync から隔離する契約コメント固定 | **done** |

Moore `|` 再挑戦は **任意最適化**（本線合格条件ではない）。

---

## 実装メモ

* client `bus::frame`: `sync_moore_cell_subscribers` — cell キーごと `declare_subscriber` + `callback` → `mpsc::sync_channel` fan-in（`try_send`；満杯は drop + `dropped_backpressure`）
* 中心ヒステリシス（x00c1）で差分 resub；初回 wire pose で center snap（declare 失敗時は `primed_pose` 未設定でリトライ）
* Interest enter: `enter_snapshot_pending` — cooldown 中は遅延、**捨てない**；明けに `fetch_snapshot_interest` + 同一 lock prune；失敗時は pending 維持；**gap→snapshot 成功時も pending クリア**（二重 Room get 回避）
* Interest exit: snapshot replace と prune を同一 world lock；center 変更時も明示 prune。**定期 prune なし**（AOI 不変時の欠落 remove ゴーストは次 enter/gap まで残りうる・意図的）
* fan-in `try_send` Full → `dropped_backpressure`；多発時はバッファ／drain 見直し（gap 定数緩和より先）
* snapshot `get` 後は `try_recv` drain（callback 背圧を解放）
* `frame_cell_wildcard_key` / `|` union は本線外（protocol rustdoc）
* NeedResync は world 非破壊 + Intent/prediction 非接触（`need_resync_is_pure_wrt_world_…` テスト）

---

## Phase

| Phase | 内容 | 状態 |
|:---|:---|:---|
| 0 | 正本 §5 + 本スライス + peer-delivery / architecture 追随 | **done** |
| 1 | x00c6a per-key Interest | **done** |
| 2 | x00c6b enter snapshot / exit prune | **done** |
| 3 | x00c6c prediction / Intent 隔離 + `done` | **done** |

---

## 完了条件

1. 既定購読が Moore per-key ≤27 であり、`cell/*` を既定本線にしない — **done**
2. AOI enter で Room snapshot により静止物を含む再可視が保証される — **done**
3. AOI exit で Moore 外ゴーストが残らない — **done**
4. NeedResync 中も Intent / prediction が継続する（別スレッド・契約コメント） — **done**
5. dual-publish 停止・LOD・`FRAME_TICK_GAP_RESYNC` 緩和・Moore `|` 本線化は非保証のまま文書化 — **done**
6. `cargo test -p alchemy-protocol` / client frame 経路 Green — **done**（protocol 95 · `bus::frame` 含む）
7. peer-delivery / architecture / spatial_interest の保証表がコードと一致 — **done**

---

## 起動・検証メモ

```powershell
# 既定（Moore per-key 本線）
# client + host；ログに Moore per-key / cell interest resub；飛散後も受信が頭打ちし、近づくと再可視

# Room オプトアウト退行確認
$env:ALCHEMY_ROOM_FRAME = "1"

cargo test -p alchemy-protocol
# from client/
cargo test -p client --bin client -- bus::frame
```

推奨ブランチ: `feat/x00c6-spatial-interest-vr-delivery-contract`
