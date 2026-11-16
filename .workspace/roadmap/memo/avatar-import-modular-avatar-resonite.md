# Alchemy メモ — アバター導入ルート（Modular Avatar / Resonite）

* **種別**: `memo/` 提案。正本ではない
* **日付**: 2026-11-16
* **関連**: [`avatar-animation-ux.md`](avatar-animation-ux.md) · [`alchemy-expansion.md`](alchemy-expansion.md) E1-1/E1-2 · [`v2.md`](../v2.md) P1 · [`spatial_interest.md`](../spatial_interest.md)（sync LOD は描画と混ぜない）
* **参考**: [Modular Avatar 紹介](https://modular-avatar.nadena.dev/ja/docs/intro)
* **目的**: VRChat 文化の Modular Avatar ルートと、Resonite 式 DnD アバター化の二本を言語化し、まず MA ルート向けの不足要素を洗い出す

---

## 1. 二つの導入ルート

| ルート | 作者体験 | Alchemy の役割 |
|:---|:---|:---|
| **Modular Avatar 文化** | Unity + MA で非破壊に衣装・ギミックを組む | 焼き上がりの消費（第一）／将来ネイティブ組立 |
| **Resonite 方式** | Blender 等で作ったアバターを画面へ DnD → VR 内でアバター化 | ランタイム装着・Rig 推定・その場で IK 対象化 |

両方とも最終的に同じランタイム（スキン / IK / シェーダー安全弁 / morph）に乗る想定。先に MA 文化ルートの土台を洗う。

---

## 2. MA ルートの解釈（採用仮説）

Modular Avatar は **Unity / NDMF 上のビルド時ツール**であり、Alchemy 実行時にそのまま動かない。

| 層 | 意味 | 当面 |
|:---|:---|:---|
| **① 文化・互換ルート** | Unity で MA 組立 → 焼き上がりを Alchemy が読める形で取り込む | **第一候補** |
| **② Alchemy 内 MA 相当** | Merge Armature / Reactive / Outfit DnD を自前実装 | 後段（Resonite ルートと共有しうる） |

やらないこと（当面）: Unity HLSL / VRCPhysBone API の完全互換、NDMF プラグイン生態系の実行時再現。

---

## 3. 取り込み契約（最初に決める分岐）

| 項目 | 問い | Alchemy 現状 |
|:---|:---|:---|
| 輸入物の形 | Unity パッケージ？ FBX+meta？ GLB？ VRM？ Alchemy Avatar Manifest？ | CAS は GLB バイト中心。Humanoid / VRM 意味論なし |
| 誰が bake するか | 作者側 Unity（MA/NDMF）か、Alchemy 変換ツールか | 変換パイプラインなし |
| MA 互換の範囲 | 「焼き上がりメッシュ＋リグ」か「MA コンポーネント意味の再現」か | 未定義 |

**メモ仮説**: 当面は ①。輸入物の第一候補は未確定（VRM 1.0 / 素 GLB＋独自 JSON / その他）。決めるまではギャップ表の「必須」を過大に広げない。

---

## 4. 現状（ギャップの出発点）

* `user_root` は Presence 1 entity の TRS。他クライアントは色付きボックス。`RenderFrame` にボーン・表情なし
* client GLB 経路は baseColor まで。`mesh.rs` は **skins / morphs 非保証**
* `SkeletonPosePage` / 非同期 PSO（E1-1 / E1-2）は expansion 目標。フル IK / SpringBone / 製品 VRM は明示的に非保証
* avatar **sync** LOD（x00c7）は中距離 `user_root` 同期のみ。描画アバター GLB とは別予算

---

## 5. 不足要素ギャップ表（MA ルート）

依存の強い順。上が無いと下は見た目だけ崩れる。

### 5.1 メッシュ基盤（最優先・ブロッカー）

| 不足 | VRChat / MA 側の前提 | Alchemy |
|:---|:---|:---|
| Skinned mesh / joints / IBM | SkinnedMeshRenderer | 未読込 |
| GPU/CPU スキニング | 毎フレーム | E1-1 構想のみ |
| BlendShape / morph | 表情・服の痩せ・Shape Changer | 未対応 |
| 複数メッシュ・複数マテリアル | アバター標準 | ワールド mesh スロット中心 |
| `user_root` への装着描画 | Avatar Descriptor | 色ボックスのみ |

→ [`avatar-animation-ux`](avatar-animation-ux.md) **段 A** + expansion **E1-1** が土台。

### 5.2 ヒューマノイド規約

| 不足 | 用途 |
|:---|:---|
| ボーン名 / Humanoid マップ | Merge Armature 互換衣装、リターゲット |
| T/A-pose・スケール正規化 | IK・クリップ共有 |
| 指ボーン | ハンドトラ・ジェスチャ |
| ViewPosition / 頭骨オフセット | 一人称カメラ（v2 P1） |

MA Setup Outfit は `Hips` 名の prefix/suffix 一致などを前提にする。後で「MA 互換衣装」を載せるなら命名規約を早めに固定する。

### 5.3 IK / トラッキング

| 不足 | 備考 |
|:---|:---|
| 3点 IK（HMD + 両手） | VR 自アバターの最低ライン |
| FBT / 追加トラッカー | 後段 |
| 視線・まぶた | AV3 / VRM lookAt 相当 |
| ハンド／フィンガートラ | OpenXR 手トラは現状非保証 |

Presentation pose は client 側（物理 Frame と分離）。host でスキニング／フル IK 評価しない（既存メモと同じ）。

### 5.4 揺れもの（PhysBone → Alchemy）

| VRChat | Alchemy で必要なもの |
|:---|:---|
| VRCPhysBone / Contacts | SpringBone 相当、または XPBD 拘束行 |
| 髪・服コライダ | 二次モーション用コライダセット |
| Merge 後の PhysBone 参照書き換え | bake 後なら不要／ネイティブ MA なら必須 |

### 5.5 シェーダー／マテリアル

| VRChat 文化 | Alchemy |
|:---|:---|
| lilToon / Poiyomi / Standard Lite / MatCap 等 | baseColor のみ |
| カスタムシェーダー | E1-2（非同期 PSO + PBR フォールバック）構想 |
| 一人称クリップ・アウトライン | なし |

**段階案**: (1) 固定 Toon/PBR 近似フォールバック → (2) MToon 相当の限定セット → (3) 検証付きカスタム WGSL。  
Unity HLSL のそのまま再生はしない（変換 or 近似）。

### 5.6 アニメ／表情／メニュー（AV3 + MA Reactive）

MA がビルド時に扱う主なもの: Animator レイヤ統合、Expression Parameters / Menu、Reactive（Object Toggle / Shape Changer / Material Setter·Swap / Mesh Cutter）。

| 不足 | 既存メモ |
|:---|:---|
| ローカル Animator / ステートマシン | 段 B |
| morph 重みチャネル | E1-1 MorphPage |
| 表情パラメータ同期方針 | 段 C/D（全ボーン送らない） |
| ジェスチャ → パラメータ | 入力マップ未整備 |
| ランタイム Toggle / Mesh Cutter | ネイティブ MA なら必要。bake 後は静的で足りる場合あり |

### 5.7 一人称・可視性

* Head chop / FirstPersonOnly / ThirdPersonOnly
* ミラー用クローン
* 影のみメッシュ

スキニングの直後に薄い契約が必要（自頭が視界を塞ぐ問題）。

### 5.8 アセット・権利・安全弁

| 項目 | 理由 |
|:---|:---|
| アバター用 decode budget | 現行 GLB 上限はワールド向け；フルアバターで足りるか要確認 |
| 着用権 / `BlobGrant` | BOOTH 文化の再配布防止（v2 Level 2） |
| NSFW フォールバック | v2 Level 3 |
| シェーダー安全弁 | 視界ジャック・AABB クリップ（v2） |
| Mute/Block → 標準アバター置換 | P1 ローカル安全弁 |

### 5.9 同期（ワイヤ）— 混ぜない

[`avatar-animation-ux`](avatar-animation-ux.md) のチャネル分離を踏襲する。

| チャネル | 置き場 |
|:---|:---|
| Locomotion / 物理権威 | Intent → host → `RenderFrame`（現行 root TRS） |
| Presentation pose | client `SkeletonPosePage` / Morph（未実装） |
| 着用アバター定義 | Manifest / Presence 近傍の blake3 参照（置き場は未決） |
| Tracking（後段） | RenderFrame にボーン全載せしない。近傍 LOD |

---

## 6. 依存順（実装に落とすときの階段）

```text
取り込み契約（GLB拡張 / VRM / 独自）
  → スキン+morph+user_root装着描画（段A + E1-1）
    → Humanoid規約 + ViewPosition
      → 3点IK
        → 固定Toon/PBRフォールバック（E1-2器）
          → SpringBone
            → 表情/Animator（段B）
              → （任意）MAネイティブ Merge / Reactive
              → Resonite式 DnD アバター化
```

「シェーダーと IK が足りない」は正しいが、**その手前にスキニングと装着描画が無い**のが現状の最大ギャップ。

---

## 7. 保証する / しない（このメモの目標）

| 保証する（目標） | 保証しない（当面） |
|:---|:---|
| `user_root` にスキン付きアバター定義を載せられる道があること | Unity HLSL / VRCPhysBone API 互換 |
| 3点 IK で自アバターが立つこと | lilToon / Poiyomi 完全パリティ |
| 見た目ポーズを物理 Frame と分離すること | MA コンポーネントの実行時解釈 |
| Resonite DnD が同じランタイム土台を共有できること | VR 内フルキーフレーム編集・製品 VRM パリティ |

---

## 8. Resonite ルート（メモのみ・後段）

* DnD import → ランタイム Rig / Humanoid 推定 → その場で IK 対象化
* MA bake 済み資産と **同一のランタイム**（skin / IK / shader 安全弁）を共有する
* ネイティブ Merge Armature / Outfit 着脱は、こちらの「空間内編集」と一緒に設計した方が重複が少ない

---

## 9. いま決めなくてよいこと

* 輸入物フォーマットの最終選定（候補だけ残す）
* Humanoid と MMD の第一級化
* MA Reactive の実行時完全再現
* 描画 LOD と avatar sync LOD の仕様溶融

次の小さな決め事: **輸入物の第一候補**（VRM 1.0 / 素 GLB＋独自 JSON / その他）を一つに絞り、ギャップ表を必須 / 後段 / やらないの三色に振る。
