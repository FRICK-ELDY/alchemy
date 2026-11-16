# Alchemy — 配信の空間インデックス（Spatial Interest）正本

* **種別**: 規模本線の仕様正本。実装第一波は [x00be](slices/x00be-spatial-interest-cell-frame-foundation.md)（キー契約 + ACL + dual-publish）。第二波は [x00bf](slices/x00bf-client-near-cell-frame-sub.md)（client 近傍セル opt-in 併読）。第三波は [x00c0](slices/x00c0-host-cell-dirty-filter.md)（host セル別 dirty / 複数セル soft-put）。第四波は [x00c1](slices/x00c1-client-cell-follow-resub.md)（自位置追従の cell 購読切替）。第五波は [x00c2](slices/x00c2-cell-exit-remove.md)（セル退出 remove）。第六波は [x00c3](slices/x00c3-multi-cell-same-tick-merge.md)（多 cell 同一 tick 合流）。第七波は [x00c4](slices/x00c4-opt-in-cell-frame-mainline.md)（opt-in cell 本線・Room frame 非購読）。第八波は [x00c5](slices/x00c5-default-cell-frame-mainline.md)（既定 cell 本線・`ALCHEMY_ROOM_FRAME` オプトアウト）。第九波は [x00c6](slices/x00c6-spatial-interest-vr-delivery-contract.md)（VR delivery 契約: Moore per-key · Interest enter 収束 · 自アバター応答）。第十波は [x00c7](slices/x00c7-avatar-sync-lod-mid-band.md)（Avatar sync LOD: 中距離 `frame/lod` · 粗い格子）
* **改善計画上の位置**: [improvement-plan-2026-11-12.md](../0_doc/evaluation/improvement-plan-2026-11-12.md) **P3-7**
* **品質本線（物理・状態）**: [mvcc_xpbd.md](mvcc_xpbd.md) Phase 1–3（dirty デルタ・SoA ページが前提）
* **現行配送契約**: [protocol/peer-delivery.md](../../protocol/peer-delivery.md)（x0024）· [architecture/client.md](../0_doc/architecture/client.md) · [architecture/protocol.md](../0_doc/architecture/protocol.md)
* **関連**: x0067（非対称 ACL）· x0069（Room 鍵 AEAD）· `HotPhysicsPage.cell_origin`（[mvcc_xpbd.md](mvcc_xpbd.md) §4.3）· 自アバター予測 x00ab

> **注記（番号衝突）**: 本 P3-7（Spatial Interest）は、roadmap README に残る旧メモ「P3-7 BLAKE3 ストリーミングはマージしない」とは**無関係**。BLAKE3 ストリーミング禁止は引き続き有効。

---

## 0. なぜ第一級仕様か

[mvcc_xpbd.md](mvcc_xpbd.md) は近傍物理品質・ゼロアロ・handover 速度復元・GPU 補間の**品質本線**である。  
同一 Room の大規模 CCU（horizon として 1,000 人）は、dirty bitmask や量子化だけでは届かない。現行は Room 単位の単一 `…/{wire_cap}/frame` を全員が購読する**フルメッシュ**であり、リレー送信と各クライアント受信が物理より先に死ぬ。

**規模の主戦場は「誰に・何を・何 Hz で送るか」（interest）である。** 本ドキュメントがその正本。

```text
品質本線: mvcc_xpbd Phase 1–3
  → dirty / AEAD デルタが使える
規模本線: spatial_interest（本ファイル）
  → セル購読分割 + avatar LOD
  → 実測ゲート 32 → 64 → 128
  → 1000 人は horizon（到達保証ではない）
VR 契約: §5（x00c6）
  → 近傍帯のみ高 Hz + Interest enter 収束 + 自アバター応答
Avatar sync LOD: §6（x00c7）
  → 中距離は分離キー frame/lod（粗い格子 · user_root · ≤5Hz）
```

---

## 1. 問題定義（現行）

| 現状 | 帰結 |
|:---|:---|
| キー式 `alchemy/…/{wire_cap}/frame` が Room 全体 1 本（[client.md](../0_doc/architecture/client.md)） | 参加者 N が増えるとファンアウトが O(N) ペイロード × O(N) 購読者 |
| host → guest の latest デルタ（x0024） | 欠落時 snapshot は正しいが、興味範囲の縮小が無いとフルメッシュのまま（x00c5 暫定 `cell/*` は飛散で破綻 → **x00c6** で Moore 境界に戻す） |
| MVCC dirty + Smallest-Three | フルメッシュ前提の**ペイロード極小化**に効く。N 人同期トポロジ自体は解かない |
| `USER_ROOT_SLOTS_MAX` DoS 天井 32 | 品質本線の到達点と 1000 人 horizon の間に段階ゲートが必要 |

粗い帯域感（アバター head+hands ≈ 3 entity、全員毎ティック更新・理想値）:

* 1000 × 3 × ~40 B ≈ 120 KB/tick → 30 Hz で受信側 ~3.6 MB/s
* 1000 購読者が同一フルストリーム → リレー送信は GB/s 級になりうる

---

## 2. 保証する / しない

| 保証する（目標・実装スライス後） | 保証しない（当面） |
|:---|:---|
| クライアントは**近傍セル群（Moore 3³）**のみ高頻度 `frame` 相当を購読できる | Phase 文書の存在だけで 1000 人 CCU を保証すること |
| **Interest enter**: 近傍に入った実在（静止含む）へ収束し、近づくと**また見える** | AOI 外に居る間の連続可視・軌跡の完全再生 |
| **Interest exit**: 現行 Moore 外の遠方ゴーストを残さない | cell 別 snapshot キーの新設 |
| **自アバター応答**: 視点・移動表示は local prediction（x00ab）；NeedResync 中も Intent / 予測を止めない | wire 遅れゼロ（reconcile は権威に収束） |
| 中距離の remote `user_root` は**低頻度 LOD**（`…/frame/lod/…` · ≤5 Hz · user_root のみ）で届く（[x00c7](slices/x00c7-avatar-sync-lod-mid-band.md)） | 1 host 上での 1000 人**相互** XPBD・全員接触 |
| 既存 Room AEAD（x0069）· wire_cap ルーティング · delivery-class（x0024）を壊さない | 確実配送 ACK / event ID チャネル |
| 物理ページの空間セルと配信キーの空間単位を**同系**に保つ（lod は同一原点の粗い格子） | `wire_cap` トピック名の秘匿 |
| 独自トランスポートを新設しない（Zenoh パイプライン維持） | マルチ権威セルの本番移譲・セル間 handover の完成 |
| | 既定本線としての `…/frame/cell/*`（x00c5 暫定・**契約外**；debug/計測のみ） |
| | 同一 `frame/cell` キー環拡大での間引き（案 A・本線禁止） |
| | Zenoh Moore `|` union 配送（x00c5 で declare OK・配送ゼロを観測；本線は per-key ≤27） |
| | 音声・メッシュ・アニメ・描画 LOD の本仕様化（別予算・非保証） |
| | lod 専用 snapshot · presence への粗位置混載 |
| | verifier Spot-Check / billing / market（独立） |

保証表の追随は [x00be](slices/x00be-spatial-interest-cell-frame-foundation.md)〜[x00c7](slices/x00c7-avatar-sync-lod-mid-band.md)（VR delivery + Avatar sync LOD 中距離）。[protocol.md](../0_doc/architecture/protocol.md) · [client.md](../0_doc/architecture/client.md) · [peer-delivery.md](../../protocol/peer-delivery.md)。dual-publish 縮小・CCU 天井拡大・描画 LOD は後続。

---

## 3. 空間単位（物理ページと配信の接合）

| 層 | 単位 | 役割 |
|:---|:---|:---|
| 物理（品質本線） | `HotPhysicsPage` + `cell_origin_* : f64`（局所 `f32` ±32 m） | XPBD・MVCC・GPU 転送 |
| 配信（規模本線） | **同一系のセル ID**（`cell_id: [i32; 3]` または origin から導出） | 近傍購読・cell publish |
| 配信 LOD（規模） | **同一原点の粗い LodCell**（`floor_div(cell, LOD_CELL_STRIDE)` · stride=3） | 中距離 sync LOD キー |
| 権威（当面） | **1 Room = 1 host** | セルは購読分割の単位であり、権威分割の単位ではない |

`cell_origin` を interest の第一級キーに昇格させる。物理ページ設計を捨てて別グリッドを発明しない。lod は別原点を作らず **同一系の stride 集約**に留める。

---

## 4. キー契約（実装固定 · x00be〜x00c7）

1. **独自トランスポート禁止**（[mvcc_xpbd.md](mvcc_xpbd.md) 🚫1 と同型）。`relay-server` / `relay-agent` / `wire_cap` ACL / Room AEAD を通す。
2. **方向 A（確定）**: セル修飾サフィックス  
   `alchemy/dev/room/{room_id}/{wire_cap}/frame/cell/{cx}_{cy}_{cz}`  
   セグメントは `format_cell_id_segment`（例: `0_0_0`, `-1_2_0`, `-1_-2_0`）。索引は shared-sim `CellId`（`CELL_SIZE_M=64` center origin）と同系。
3. Room 全体 `…/frame` は **互換・tick 正本として残す**（x00be: host dual-publish；**x00c5**: client 既定は cell 本線で本キーを**購読しない**；`ALCHEMY_ROOM_FRAME=1` で Room のみへオプトアウト）。`…/snapshot` はセル分割しない（欠落再同期・**Interest enter 収束**の正本）。
4. **x0067**: ACL は `…/frame/cell/*` および（x00c7）`…/frame/lod/*` を **明示**（`frame/**` 一括禁止）。**AEAD AAD** はキー式非束縛のまま（x0069 据え置き）。
5. delivery-class（latest / snapshot / best-effort）の語彙は [peer-delivery.md](../../protocol/peer-delivery.md) を継承し、「frame = latest・欠落は Room snapshot」を維持する。lod も latest（lod 専用 snapshot は作らない）。
6. **x00bf**: client は（当時）`ALCHEMY_CELL_FRAME=1` のとき origin Moore 近傍（3³）の cell キーを購読開始（初期中心）。歴史的には Room と併読；**x00c4** で opt-in は cell 本線へ；**x00c5** で既定 cell。
7. **x00c0**: host は dirty upsert を `CellId` でバケットし、非空セルへ**セル別密封**で soft-put（seal/put soft-fail・publisher LRU 64）。Room は全文維持。Room-only 時の同 tick 上書きは `decide_room_frame_tick`（x00c5 以降は `ALCHEMY_ROOM_FRAME=1`）。
8. **x00c1 / x00c6**: cell 本線時、Presence 割当後の **wire** `local_user_root` 位置で中心 `CellId` を追従（`CELL_HYSTERESIS_M`；**活性時も ≤250ms deadline**）。中心変化で Moore per-key を差分 resub。
9. **x00c2**: host は `last_wire_cell` で前回配信セルを追跡し、境界横断・Room removes 時に**旧セル**へ remove をセル別密封 soft-put（remove-only セル含む）。Room 全文 removes は維持。
10. **x00c3**: client cell 経路は `decide_cell_frame_tick` で同一 tick の複数セルを合流適用（跨 cell は upsert guard；単一 frame 交差は remove 勝ち）。境界横断は Room 無しで収束。
11. **x00c4**: `ALCHEMY_CELL_FRAME=1` 時は Room `…/frame` を**購読せず** cell Moore を tick 本線とする。gap→resync は cell が Room `…/snapshot` を get。当時の既定は Room-only。host dual-publish 維持。
12. **x00c5**: client 既定は cell 本線（Room frame 非購読）。当時の暫定購読は `…/frame/cell/*`（規模本線と矛盾する interim）。歴史的 Moore `|` union は local peer で declare OK・配送ゼロのため退避。`ALCHEMY_ROOM_FRAME=1` で Room のみ。`ALCHEMY_CELL_FRAME` legacy。host dual-publish 維持。
13. **x00c6（§5）**: 既定本線を **Moore per-key ≤27 subscriber** に戻す。`cell/*` は debug/計測専用。Interest enter → rate-limited Room snapshot。Interest exit prune。自アバター予測を配送契約へ昇格。dual-publish 停止は更に後続（enter が Room snapshot に依存するため）。
14. **x00c7（§6）**: 中距離 Avatar sync LOD。キー `…/frame/lod/{lx}_{ly}_{lz}`（`LodCell = floor_div(CellId, 3)`）。host は LodCell ごとの **occupied 名簿 + exit removes** を ≤5 Hz soft-put。client は LodCell Moore ≤27 を購読し **near 優先 apply**；epoch は pose のみ（jitter 非接触）。同一 `frame/cell` 環拡大での間引き（案 A）は本線禁止。描画 LOD・CCU 天井拡大は非対象。

実装第一波: [x00be-spatial-interest-cell-frame-foundation.md](slices/x00be-spatial-interest-cell-frame-foundation.md)。  
実装第二波: [x00bf-client-near-cell-frame-sub.md](slices/x00bf-client-near-cell-frame-sub.md)。  
実装第三波: [x00c0-host-cell-dirty-filter.md](slices/x00c0-host-cell-dirty-filter.md)。  
実装第四波: [x00c1-client-cell-follow-resub.md](slices/x00c1-client-cell-follow-resub.md)。  
実装第五波: [x00c2-cell-exit-remove.md](slices/x00c2-cell-exit-remove.md)。  
実装第六波: [x00c3-multi-cell-same-tick-merge.md](slices/x00c3-multi-cell-same-tick-merge.md)。  
実装第七波: [x00c4-opt-in-cell-frame-mainline.md](slices/x00c4-opt-in-cell-frame-mainline.md)。  
実装第八波: [x00c5-default-cell-frame-mainline.md](slices/x00c5-default-cell-frame-mainline.md)。  
実装第九波: [x00c6-spatial-interest-vr-delivery-contract.md](slices/x00c6-spatial-interest-vr-delivery-contract.md)。  
実装第十波: [x00c7-avatar-sync-lod-mid-band.md](slices/x00c7-avatar-sync-lod-mid-band.md)。

---

## 5. VR delivery contract（x00c6）

VR 体験に関わる配送の第一保証。Interest 境界と自アバター応答を**同一正本**で扱う。実装: [x00c6](slices/x00c6-spatial-interest-vr-delivery-contract.md)。

### 5.0 決めた方針

| 項目 | 契約 |
|:---|:---|
| 第一保証 | Interest 境界 **と** 自アバター応答 |
| 高 Hz 対象 | **近傍帯のみ**（Moore 3³ · `CELL_SIZE_M=64`） |
| AOI 外の飛散物 | 高 Hz・可視を**保証しない** |
| 近づいたとき | **また見え始めることを保証**（Interest enter 収束） |
| 暫定 `…/frame/cell/*` | **契約から外す**（既定本線禁止） |

```text
MoveIntent (30Hz) ──► host authority
       │
       └────────────► local prediction (x00ab) ──► eye / locomotion display
                              ▲
Moore near cells (≤27) ──delta apply──┘
       │
       └─ AOI enter ──► Room …/snapshot (rate-limited) ──► replace converge
```

### 5.1 Interest（誰が何を高 Hz で受けるか）

* Client の frame 本線購読集合 = wire `local_user_root` 中心の **Moore 3³**（`moore_neighborhood_cells`）。
* **購読形（採用）**: cell キーごと **最大 27 subscriber**。中心ヒステリシスで差分 resub（x00c1 の追跡を購読に戻す）。
* Zenoh `|` union は x00c5 で declare OK・配送ゼロが再現済みのため**本線にしない**。per-key Green 後の任意最適化候補に留める。
* `frame_cell_wildcard_key`（`…/frame/cell/*`）は **debug / 計測専用**。既定本線に使わない。
* Host は現行どおり dirty セルへ soft-put（到達非保証）。Room `…/frame` は dual-publish の tick 正本として当面維持（client 既定は非購読のまま可）。

### 5.2 Interest enter / exit（見え始め・見え終わり）

* **Enter（保証）**: Moore 集合に新しい cell が 1 つでも入ったとき、rate-limited で Room `…/snapshot` を get し entity 表を置換収束する。**cooldown 中の enter は捨てない**（`enter_snapshot_pending` で遅延し、cooldown 明けに必ず 1 回撮る）。静止物もこれで「また見える」。cell 別 snapshot キーは作らない。
* **Exit（保証）**: snapshot 置換は Interest prune と**同一 world lock**；AOI 変更時（center 変化）も明示 prune で現行 Moore 外の遠方ゴーストを残さない。
* **保証しない**: AOI 外に居る間の連続可視・軌跡の完全再生。**AOI 不変時**に host cell remove が欠落した遠方ゴーストが、次の enter / gap→snapshot まで残ること（定期 prune はしない；受信予算優先）。

### 5.3 Tick / 欠落（multi-cell）

* Host `net_tick` は Room 正本。cell put は同一 tick を共有（現行）。
* Client は `decide_cell_frame_tick` + x00c3 合流を維持。
* 受信量上限は Interest で先に抑える。`FRAME_TICK_GAP_RESYNC=1` の緩和は本契約では必須にしない（enter 時 snapshot と局所予測で体験を守る）。gap 嵐が残る場合は実装波で計測後に検討。

### 5.4 自アバター応答

* Display pose: owned local root / held Grab は prediction 優先（x00ab · 現行 scene 経路を契約化）。
* `ApplyFrameOutcome::NeedResync` / snapshot cooldown 中も **MoveIntent publish と prediction tick は継続**。
* 「入力遅延→停止」を Interest 過負荷の症状として受け入れない（回帰観点）。

### 5.5 意図的にやらないこと（本契約）

* 応急の `ALCHEMY_ROOM_FRAME=1` 固定や gap 定数の場当たり緩和を「本線修正」と呼ぶこと
* cell 別 snapshot キーや独自トランスポートの新設
* dual-publish 停止の同時実施（enter 収束が Room snapshot に依存するため、停止は enter 代替が出来てから）
* 認証・AEAD・検証の緩和

---

## 6. Avatar sync LOD（x00c7）

規模本線の中距離同期。近傍品質（§5）を落とさず、共有 pub/sub のまま帯域を分ける。実装: [x00c7](slices/x00c7-avatar-sync-lod-mid-band.md)。

### 6.0 決めた方針

| 項目 | 契約 |
|:---|:---|
| スコープ | **sync LOD のみ**（描画 GLB / スキン / impostor / ボーンは別予算） |
| 配送モデル | **キー分離（案 B）** — 近傍 `…/frame/cell/…` と中距離 `…/frame/lod/…` |
| 中距離格子 | 同一原点 · `LOD_CELL_STRIDE = 3`（≈192 m）· `LodCell = floor_div(CellId, 3)` |
| 中距離購読 | LodCell 中心 Moore 3³ **per-key ≤27** |
| ペイロード | 現行 `RenderFrame` / `Entity` · **user_root のみ** · LodCell ごと **occupied 名簿 + exit removes** · ≤5 Hz |
| 適用優先 | 同一 `entity_id` は **near（cell）が正**。lod は近傍に無い remote user_root 専用 |
| 表示クロック | lod / AOI prune の epoch 反映は **pose のみ**（sim jitter は near Applied Frame 専用） |
| 遠方 | lod/cell 追加購読なし。`…/presence` は割当のみ（粗位置を載せない） |

**案 A（同一 `frame/cell` 環拡大＋間引き）を本線にしない理由:** cell キーは視聴者別チャネルではなく空間共有バケットである。近傍クライアントが高 Hz 全文を必要とするキーに間引きを載せられず、購読環だけ広げると受信が増えるだけで sync LOD にならない。

```text
Near: CellId 64m · Moore ≤27 · full TRS dirty · ~30Hz   (§5)
Mid:  LodCell stride=3 · Moore ≤27 · user_root only · ≤5Hz
Far:  no lod/cell subscribe · presence assignment only
```

| 帯 | 目安 | 同期内容 |
|:---|:---|:---|
| 近傍 | 同一セル + 隣接（Moore · 64 m） | フル TRS（必要なら将来 hands）高 Hz・dirty デルタ（§5） |
| 中距離 | LodCell Moore（≈192 m 格子） | `…/frame/lod/…` · user_root 名簿 + exit removes · ≤5 Hz（x00c7） |
| 遠方 | mid AOI 外 | 高 Hz 非保証（§5.2）。presence 割当のみ |

音声・アバターメッシュ・表情・描画 LOD は**本正本の保証外**（別予算）。物理 Grab / 接触は近傍帯に閉じる（品質本線の XPBD と整合）。lod 専用 snapshot は作らない。Interest enter/exit（Room snapshot）は near のみ。

### 6.1 意図的にやらないこと（本契約）

* 同一 `frame/cell` キーへの間引き載せ（案 A）
* lod / cell 別 snapshot、presence への粗位置混載
* `USER_ROOT_SLOTS_MAX` 引き上げ・G2 合格宣言
* dual-publish 停止の同時実施
* 描画 LOD と sync LOD の仕様溶融、ボーンの `RenderFrame` 混入

---

## 7. 権威モデル

| 段階 | モデル | 規模への効き |
|:---|:---|:---|
| **当面（本正本の実装第一波）** | 1 Room = 1 untrusted host。セルは**購読・publish 分割**のみ | フルメッシュ解除が主効果 |
| **後段 horizon** | 論理ワールド複数セルのまま、必要ならマルチ権威 | host CPU 限界を超えるときのみ |

メガ空間は「権威を先に割る」より「**購読を先に割る**」。マルチ権威セル間の一貫性・移譲は更に後段とし、品質本線の handover（速度復元）と混同しない。

---

## 8. マイルストーン（実測ゲート）

現行 DoS 天井 `USER_ROOT_SLOTS_MAX = 32` を起点にする。各段で **帯域・host CPU・client 受信**を測り、次段の天井拡大条件にする。

| ゲート | CCU 目安 | 合格の意味（例） |
|:---|:---|:---|
| G0 | ≤32 | 現行契約 + MVCC Phase 3 デルタで安定 |
| G1 | 64 | interest（近傍セル購読）無しでは不合格になりうることを計測で示すか、interest 試作で通過 |
| G2 | 128 | cell 購読 + avatar LOD が本線。フルメッシュ禁止を運用で固定 |
| Horizon | 1000 | interest・予算・（必要なら）権威拡張が揃った**将来目標**。到達保証ではない |

「MVCC Phase 完了 = 1000 人」と書かない。規模 KPI は本ファイル、品質 KPI は [mvcc_xpbd.md](mvcc_xpbd.md)。

---

## 9. 依存と着手ゲート

| 依存 | 理由 |
|:---|:---|
| [mvcc_xpbd.md](mvcc_xpbd.md) **Phase 3** DoD Green | dirty デルタ・host 権威ループ・AEAD 配信が品質として使えること |
| x0024 delivery-class | latest / snapshot 再同期語彙 |
| x0067 / x0069 | ACL・部屋機密を壊さない拡張 |
| x00ab owned prediction | 自アバター応答の実装前提（§5.4） |
| 独立 | P3-1 verifier · P3-3 S2S · P3-4 billing · Dual-AST Kernel / OSC |

| 作業 | 条件 |
|:---|:---|
| **本ファイル（アウトライン正本）** | いつでも。MVCC Phase と**並走可** |
| **実装スライス群** | **MVCC Phase 3 DoD Green 後**。第一波 [x00be](slices/x00be-spatial-interest-cell-frame-foundation.md)〜第十波 [x00c7](slices/x00c7-avatar-sync-lod-mid-band.md) **done**（§5 VR delivery · §6 Avatar sync LOD）。dual-publish 縮小・CCU 天井拡大・G1 実測は後続 |

---

## 10. やらないこと

* 独自 UDP / quinn / WebTransport で帯域を稼ぐ
* 検証スキップ・Room AEAD / ManifestSeal バイパス
* MVCC だけで大規模 CCU を宣言する文書・PR 説明
* フルメッシュのまま Phase を増やして「スケールした」ことにする
* 既定本線として `…/frame/cell/*` を規模解決と呼ぶこと（受信量が dirty cell に線形）
* 同一 `frame/cell` 環拡大を Avatar sync LOD と呼ぶこと（案 A）
* 本アウトラインだけで architecture 保証表やキーヘルパを破壊的に書き換える（実装スライスで契約固定）
