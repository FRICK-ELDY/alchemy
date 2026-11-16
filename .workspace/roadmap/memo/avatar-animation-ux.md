# Alchemy メモ — アバターアニメ／ダンス／MV 体験の置き場

* **種別**: `memo/` 提案。正本ではない
* **関連**: [`alchemy-expansion.md`](alchemy-expansion.md) E1-1（`SkeletonPosePage`）· [`spatial_interest.md`](../spatial_interest.md) avatar sync LOD · overview の audio/tracking
* **目的**: Unity 風アニメ・ダンスワールド・部屋内 MV 撮影を、現行の定義／権威／描画／規模の境界にどう乗せるかを言語化する。UX の正解は未確定のため、先に「混ぜないチャネル」と段階だけ固定する

---

## 1. 現状認識

* プレイヤーは Presence の `user_root` 1 entity の TRS。他クライアントは色付きボックス。`RenderFrame` は entity TRS のみ（ボーン・クリップ・表情なし）
* リップシンク等の morph は「アバター定義＋client 実行」として想像しやすい
* ダンスを音楽キーフレームに合わせる／VR 内でキーフレーム編集する／部屋で MV 撮影する、は別体験であり同期モデルも違う
* 土台段階では作者向けタイムラインより、後から選べる薄い契約を残す方が先

---

## 2. 体験の3段（混ぜない）

| 段 | ユーザーがやること | 必要なもの | VRキーフレーム |
|:---|:---|:---|:---|
| **見る・踊る** | 同じ曲に合わせて踊る／見る | クリップ再生＋薄い時刻同期＋音 | 不要 |
| **撮る** | 角度・ライティング・演出で録画 | カメラ／リプレイ／エクスポート | ほぼ不要 |
| **作る** | 振り・表情・カメラを編集 | タイムライン／曲線／リターゲット | 本丸（当面 Desktop 可） |

ダンスワールドの魅力の多くは「見る・踊る」＋任意の「撮る」で足りる。振りは外製クリップを同じ `t0` でローカル再生する、という仮説を第一に置く。

---

## 3. 姿勢チャネル（ワイヤとメモリ）

| チャネル | 中身 | 置き場 |
|:---|:---|:---|
| Locomotion / 物理権威 | root TRS（将来 head/hands） | Intent → host → `RenderFrame`。Grab・セル interest と直結 |
| Presentation pose | ボーン・morph | client `SkeletonPosePage` / Morph（物理ページと分離・E1-1） |
| Clip / 演出同期 | `clip_id` (blake3) + start tick / rate / loop | 別キー（`…/anim` 等）。ボーン全送しない |
| Tracking（後段） | フルトラ・表情ストリーム | overview どおり RenderFrame に混ぜない。近傍 LOD |

やらないこと（当面）: ボーン配列を `RenderFrame` に載せる。描画 LOD と avatar sync LOD を同一仕様に溶かす。host でスキニング／クリップ評価する。

---

## 4. 層ごとの置き場

| 層 | 置く | 置かない（当面） |
|:---|:---|:---|
| assets | メッシュ／リグ契約／AnimationClip（CAS） | ライブ再生時刻の権威、検疫ヒューリスティック本体 |
| protocol | 薄い `AnimPlayState`・将来 tracking schema | Humanoid/MMD 意味論、GLB バイト |
| host / shared-sim | root 物理・（任意）再生メタの配布 | スキニング、SpringBone、クリップ評価 |
| client | 再生器・スキニング・OSC 入口・録画消費 | 部屋権威の物理 |
| world | 着用／配置の権利ゲート | アニメ再生ロジック |
| 作者 UI | Dual Editor / Desktop を第一候補 | VR 内フルキーフレームを必須にしない |

---

## 5. 段階（本線と番号を共有しない）

| 段 | 内容 | ねらい |
|:---|:---|:---|
| A | remote root に静的 GLB／ローカル idle。ワイヤは現行 TRS | ボックス→人型 |
| B | client ローカル Animator（自アバター、未同期） | 再生エンジン固定 |
| C | `AnimPlayState` の薄い同期（同一クリップ再生） | ダンスワールドの本丸 |
| D | 近傍 tracking / 圧縮スケルトン（avatar sync LOD） | フリーダンス・フルトラ |
| E | retarget・表情製品・SpringBone 等 | expansion 当面非保証の先 |

撮影体験は C の消費者として後付け可能（リプレイ＝記録された pose / play state の再生）。本格タイムラインは E 近傍でも、最初は Desktop。

---

## 6. 保証する / しない（このメモの目標）

| 保証する（目標） | 保証しない（当面） |
|:---|:---|
| 見た目ポーズを物理 Frame と分離して扱えること | VR 内キーフレーム編集器の完成 |
| ダンス第一仮説が「定義＋時刻」であること | フルスケルトン 30Hz 全員配信 |
| リップ／表情をアバター morph 帯として扱えること | MMD/Humanoid 製品パリティ、規約判断の自動化 |
| 外製クリップ＋部屋内カメラで「撮る」道を塞がないこと | Alchemy が Unity Animation ウィンドウを置き換えること |

---

## 7. いま決めなくてよいこと

* Humanoid と MMD の第一級化
* VR タイムラインの操作感
* SpringBone / フル IK / 製品 VRM
* 作者ツールを Dual-AST ギミックに乗せる詳細

最初の成功体験の仮説: **外で作った振りを、同じ曲で、複数人で揃え、あとからカメラで撮る**。キーフレーム編集は「足りない」と実測されてからでよい。
