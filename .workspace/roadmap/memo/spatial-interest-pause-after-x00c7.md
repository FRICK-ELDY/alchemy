# Alchemy メモ — 規模本線（Spatial Interest）を x00c7 後でいったん止める

* **種別**: `memo/` 作業メモ。正本ではない
* **日付**: 2026-11-16
* **関連**: [`spatial_interest.md`](../spatial_interest.md) · [`slices/x00c7-avatar-sync-lod-mid-band.md`](../slices/x00c7-avatar-sync-lod-mid-band.md) · [`slices/x00c6-spatial-interest-vr-delivery-contract.md`](../slices/x00c6-spatial-interest-vr-delivery-contract.md) · roadmap [`README.md`](../README.md)（次空き `x00c8`）
* **目的**: x00be〜x00c7 までの到達点と、「テストプレイヤー待ちで規模本線を pause し別レーンへ移る」判断を残す

---

## 1. 到達点（pause 時点）

| 波 | 内容 | 状態 |
|:---|:---|:---|
| x00be〜x00c5 | cell frame 契約 → dirty → follow → exit remove → merge → 既定 cell 本線 | **done** |
| x00c6 | VR delivery（Moore per-key ≤27 · Interest enter/exit · 自アバター予測） | **done** |
| x00c7 | Avatar **sync** LOD（`frame/lod` · LodCell stride=3 · 名簿+exit removes · pose-only epoch） | **done** |

規模本線の「購読分割 → VR delivery → mid-band sync LOD」までは契約・実装とも一段落。1000 人 horizon / G2 天井拡大は未実施（意図的）。

---

## 2. なぜここで止めるか

* 次の規模アクションはどれも **実測** か **重い設計ゲート** に寄る
  * G0: near only vs near+lod の受信／送信バイト比較 + mid 色ボックス手動確認（x00c7「計測」pending）
  * host dual-publish 縮小: Interest enter がまだ Room `…/snapshot` 依存 → **enter 代替を決めてから**。いきなり止めない
  * G2 / CCU 天井: **実測メモ + dual-publish 方針**が揃ってからの別判断
* 上記のうち G0 計測は **テストプレイヤーがそろわないと進みにくい**
* 実装を増やして天井だけ上げるのは正本のゲート順と逆

→ 規模本線は **paused**（捨てない）。プレイヤー待ちのあいだは別レーンへ。

---

## 3. 再開時の順序（規模に戻るとき）

1. **G0 計測メモ**（天井据え置き · `USER_ROOT_SLOTS_MAX=32`）
   * Observer 原点固定、Mover を mid（目安 ~200–300 m）へ
   * 手動: mid 色ボックスが低頻度更新 / far で消える / near で高 Hz 復帰
   * 数値: host `physics:` の `cell_ok` / `lod_ok`（~60s）+ lod ciphertext 平均で増分概算
   * 現行に lod 購読オフ env は無い → near-only 厳密比較は一時パッチか「lod 増分」切り出し
2. **dual-publish 縮小の設計**（enter 代替）→ 段階実装。Room put 一括停止は禁止
3. **G2 / CCU 天井**は 1–2 の後の別判断

---

## 4. pause 中の別レーン

人数なしで進められる x00c8 候補（roadmap 目次）:

* Spatial VCS / Save As / Undo UI
* WorldRoot Lens 同期
* （任意）dual-publish enter 代替の**設計だけ** — 実装で dual-publish を止めない

規模に戻るトリガ: テストプレイヤーがそろった / G0 メモを残した / enter 代替案が文書で固定された。

---

## 5. やらないこと（pause 中）

* `USER_ROOT_SLOTS_MAX` 引き上げや G2 合格宣言
* Interest enter 代替なしの dual-publish 停止
* 同一 `frame/cell` 環拡大での間引き（案 A）
* 描画 LOD と sync LOD の仕様溶融
* 「計測なしで規模が終わった」と読むこと（x00c7 計測は pending のまま）
