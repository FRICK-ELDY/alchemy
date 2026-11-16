# x00c7 — Spatial Interest: Avatar sync LOD（中距離 `frame/lod` · 粗い格子）

> **ID:** `x00c7` · **slug:** `avatar-sync-lod-mid-band` · **status:** `done` · **area:** `[protocol, relay, host, client, docs]` · **depends_on:** `[x00c6]`  
> **優先度:** [spatial_interest.md](../spatial_interest.md) 実装第十波（P3-7 · §6 Avatar sync LOD）  
> **後続:** G1 帯域実測 · G2 / CCU 天井拡大（別判断）· host dual-publish 縮小（enter 代替が出来てから）· 描画 LOD / アバターメッシュ（別予算）

目標: 近傍帯（x00c6 Moore `…/frame/cell/…`）の品質を落とさず、中距離の remote `user_root` を **分離キー** `…/frame/lod/…` で低 Hz 同期する。描画（GLB / スキン / impostor）は対象外。同一 64 m 格子の購読環拡大（案 A）は本線にしない。

根拠: [spatial_interest.md](../spatial_interest.md) §6 · [peer-delivery.md](../../../protocol/peer-delivery.md) · x00c6 · x0024 · x0067

---

## 着手ゲート（満たしていること）

| 条件 | 状態 |
|:---|:---|
| x00c6 VR delivery（Moore per-key · enter · 自アバター） | **done** |
| Room AEAD / wire_cap / x0024 を壊さない | 本スライスで **契約維持** |
| 独自トランスポート禁止 | 維持（Zenoh のみ） |
| [spatial_interest.md](../spatial_interest.md) §6 契約化 | Phase 0 で固定 |

破ってはならないこと: Room AEAD / ManifestSeal / 検証スキップ、`frame/**` ACL 化、cell / lod 別 snapshot キー新設、dual-publish 停止、近傍 Moore ≤27 の破棄、同一 `frame/cell` キーへの間引き載せ（案 A）、`USER_ROOT_SLOTS_MAX` 引き上げ / G2 合格宣言、確実配送 ACK 新設、ボーン配列の `RenderFrame` 混入、描画 LOD との仕様溶融。

---

## 決めた方針

| 項目 | 契約 |
|:---|:---|
| スコープ | **Avatar sync LOD のみ**（描画・表情・hands tracking 非対象） |
| 配送モデル | **キー分離（案 B）** — 近傍 `…/frame/cell/…` と中距離 `…/frame/lod/…` |
| 中距離格子 | 同一原点 · `LOD_CELL_STRIDE = 3`（1 LodCell ≈ 192 m）· `LodCell = floor_div(CellId, 3)` |
| 中距離購読 | LodCell 中心の **Moore 3³ per-key ≤27**（64 m 殻の +98 購読は採用しない） |
| ペイロード | 現行 `RenderFrame` / `Entity` 再利用。upsert は **user_root のみ** · ≤5 Hz |
| 遠方 | lod / cell 追加購読なし。既存 `…/presence` は割当のみ（粗位置を載せない） |
| 適用優先 | 同一 `entity_id` は **near（cell）が正**。lod は近傍に無い remote user_root 専用 |

```text
Near (x00c6):  CellId 64m  · Moore ≤27 · full TRS dirty · ~30Hz
Mid  (x00c7):  LodCell = floor_div(CellId, 3) · Moore ≤27 · user_root only · ≤5Hz
Far:           no lod/cell subscribe · presence assignment only
```

**案 A を本線にしない理由:** `frame/cell/{cx}_{cy}_{cz}` は視聴者別チャネルではなく空間共有バケットである。近傍クライアントが高 Hz 全文を必要とする cell に間引きを載せられない。購読環だけ広げると受信が増えるだけで sync LOD にならない。

---

## 保証する / しない（本スライス）

| 保証する | 保証しない |
|:---|:---|
| 近傍帯（Moore `frame/cell` · enter/exit · x00ab）を回帰させない | アバターメッシュ / 表情 / ボーン / hands |
| 中距離で remote `user_root` の粗位置が **≤5 Hz** で届く経路 | lod put / cell put の到達 |
| `…/frame/lod/{lx}_{ly}_{lz}` キー + WAN ACL 明示（`frame/**` 禁止） | lod 専用 snapshot · 軌跡完全再生 |
| lod と cell の帯域・購読予算が分離（各 Moore ≤27） | dual-publish 停止 · フルメッシュ廃止 |
| 描画が色ボックスのままでも sync 契約を検証できる | `USER_ROOT_SLOTS_MAX` 拡大 · G2 合格宣言 |
| — | 音声 · Moore `|` · `FRAME_TICK_GAP_RESYNC` 緩和 |
| — | presence への粗位置混載 |

---

## 実装波

| 波 | 内容 | 状態 |
|:---|:---|:---|
| **x00c7a** | 契約文書 + protocol ヘルパ（`LOD_CELL_STRIDE` / `cell_to_lod_cell` / `frame_lod_key`）+ WAN ACL `frame/lod/*` + 単体テスト | **done** |
| **x00c7b** | host: occupied user_root → LodCell バケット ≤5 Hz 密封 soft-put | **done** |
| **x00c7c** | client: lod Moore 購読 + near 優先 apply + lod-Moore 外 user_root prune。x00c6 経路 Green | **done** |
| **計測** | G0 内で near only vs near+lod の受信バイト比較メモ（天井は上げない） | **pending**（本線実装の合格条件外・別メモ） |

---

## 契約詳細

### キー

* `alchemy/dev/room/{room_id}/{wire_cap}/frame/lod/{lx}_{ly}_{lz}`
* セグメントは `format_cell_id_segment` 流用（例: `0_0_0`, `-1_2_0`）
* `lx,ly,lz = floor_div(cx,cy,cz, LOD_CELL_STRIDE)`（`LOD_CELL_STRIDE = 3`）
* protocol: `cell_to_lod_cell` / `frame_lod_key` /（任意）`frame_lod_wildcard_key`（ACL・debug）
* lod 中心の Moore は既存 `moore_neighborhood_cells` を LodCell 座標に適用
* WAN ACL: `alchemy/dev/room/*/*/frame/lod/*` を host-put / host-egress-declare / guest-sub / guest-egress に明示（x0067 同型）

### ペイロード

* 型: 現行 `RenderFrame` / `Entity`（新 proto なし）
* upsert: **user_root のみ**（世界 mesh・Grab 対象は載せない）
* TRS: root 全文可（自由度削減は将来。本スライスの主効果は種別フィルタ + Hz）
* `acked_intent_seq`: Room のみ。lod では空または受信側無視
* delivery-class: **latest**。lod 欠落の snapshot は作らない

### Host publish

* Room `…/frame` + cell dirty soft-put は現行維持
* 追加: occupied user_root を LodCell バケットし **≤5 Hz**（壁時計 ≈200 ms）で `frame/lod/…` へセル別密封 soft-put
* **ペイロード意図**: LodCell ごとの **occupied 名簿**（dirty デルタではない）。前回 publish 集合との差分で **exit removes**（空 LodCell は remove-only）を同梱
* seal/put 失敗は soft-fail（cell と同型）。**成功した put があるときだけ** `last_lod_put_at` を進める
* lod publisher LRU は cell と隔離
* client: lod / AOI prune 後は `ingest_applied_world_poses`（mirror/mvcc の TRS のみ。**jitter 非接触** — 近傍 30Hz 表示クロックを汚染しない）。lod `try_recv` drain は **tick 昇順ソート** → world 適用をまとめたあと **pose ingest 1 回**（LodCell 毎の PublishedPair 回転を避ける）。同一 tick の LodCell 横断は **x00c3 相当合流**（upsert guard；within-frame upsert∩remove → remove wins）。near 優先。lod removes 適用

### Client subscribe / apply

* 近傍: `sync_moore_cell_subscribers` 不變（x00c6）
* 中距離: wire `local_user_root` の fine `CellId` → `LodCell` 中心で LOD-Moore ≤27 を per-key 購読。ヒステリシスは fine cell 追従；lod 中心は fine 中心から導出
* **適用優先**: 近傍 world に居る／直近 near frame で触れた `entity_id` は lod で上書きしない
* Interest enter/exit（Room snapshot · Moore prune）は **near のみ**
* lod 退出: lod-Moore 外の **user_root のみ** prune（世界静的物は触らない）
* NeedResync / MoveIntent / prediction は x00c6c のまま（lod 経路は非接触）

---

## Phase

| Phase | 内容 | 状態 |
|:---|:---|:---|
| 0 | 正本 §6 + 本スライス + peer-delivery / architecture / README 追随 | **done** |
| 1 | x00c7a キーヘルパ + ACL | **done** |
| 2 | x00c7b host lod publish | **done** |
| 3 | x00c7c client lod interest + `done` | **done** |

---

## 完了条件

1. 正本 §6 / peer-delivery / architecture が本スライスのキー分離契約と一致する — **done**
2. 案 A（同一 `frame/cell` 環拡大での間引き）を本線にしない理由が文書化されている — **done**
3. `frame_lod_key` が `…/frame/lod/{lx}_{ly}_{lz}` を返し、`cell_to_lod_cell` が stride=3 で往復する — **done**
4. WAN ACL に `frame/lod/*` 明示があり、`frame/**` 化していない — **done**
5. host が occupied user_root を ≤5 Hz で lod soft-put する — **done**
6. client が lod Moore ≤27 を購読し、near 優先 apply・lod prune する — **done**
7. x00c6 経路（Moore cell · enter · prediction）が Green のまま — **done**（`bus::frame` 21）
8. dual-publish 停止・CCU 天井拡大・描画 LOD は非保証のまま文書化 — **done**

---

## 起動・検証メモ（実装波）

```powershell
cargo test -p alchemy-protocol
# from client/
cargo test -p host-server --bin host-server
cargo test -p client --bin client -- bus::frame

# 既定: near Moore +（x00c7c 後）lod Moore。ログに lod soft-put / lod interest resub
# Room オプトアウト退行
$env:ALCHEMY_ROOM_FRAME = "1"
```

推奨ブランチ: `feat/x00c7-avatar-sync-lod-mid-band`
