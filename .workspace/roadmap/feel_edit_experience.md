# Alchemy — 体感・編集体験本線（Feel / Edit）

* **種別**: **体験本線**の優先度・段階正本（品質本線枠クローズ後の「配線・操作感・公式 UI・拡張土台」）
* **前提**: 品質本線 [scene_store_sim_arena.md](scene_store_sim_arena.md) Phase 1–3 枠は [x00e9](slices/x00e9-rigid-solver-boundary.md) で **done / クローズ**（器はある。体感経路の多くは未配線）
* **規模本線**: [spatial_interest.md](spatial_interest.md)（本ファイルと混同しない）
* **UI 第一原理**: [alchemy_zero_base_spatial_ui_lens_uix_architecture.md](alchemy_zero_base_spatial_ui_lens_uix_architecture.md)（Spatial Lens / Uber-SDF / Zero-String / CustomUIX）
* **拡張ギャップ（ヒッチ・IME）**: [memo/alchemy-expansion.md](memo/alchemy-expansion.md)（**E1 / E2** — 本線番号と非共有）
* **アバター見た目・ダンス**: [memo/avatar-animation-ux.md](memo/avatar-animation-ux.md)（本線番号と非共有）
* **既存 Facet 最小**: [slices/x000f-dash-facet-minimal.md](slices/x000f-dash-facet-minimal.md)（語彙のみ。並べ替え・ユーザー追加は非保証）
* **空きスライス ID**: **`x00ea`** 以降（発行時は [README.md](README.md) と当該 `slices/` を更新）

### 番号規約

品質本線 Phase 1a–3・規模本線・improvement P0〜・Expansion **E1/E2** と **番号を共有しない**。本正本は次の帯のみ使う。

| 帯 | 意味 |
|:---|:---|
| **F** | Feel wiring — Scene Store / Sim Arena 契約を Ring3・操作まで通す |
| **L** | Laser — Resonite 流インタラクションレーザー |
| **T** | Text — Zero-String / MSDF / CJK / 文言カタログ |
| **G** | Glass — 公式ダッシュの本番見た目（安価ガラス） |
| **Fc** | Facet — ダッシュにモジュールを足せる土台（UIX 中身より先） |
| **U** | UIX — 誰でも編集できるパネル中身（CustomUIX） |
| **E** | Expansion — [memo/alchemy-expansion.md](memo/alchemy-expansion.md) のまま（ヒッチ根絶・IME・CAS GC） |

---

## 1. 目的と非目的

### 目的

1. 品質本線で置いた **器（Scene Store / Sim Arena / SoftLease / PendingLocalOps / AvatarRig 境界 / RigidSolver 境界）を、遊べる・編める経路まで配線**する。
2. VR レーザー・公式ダッシュ見た目・文字・Facet 装着を、**Resonite 相当の体験目標**として段階化する（クローン実装はしない）。
3. 「やることが多すぎる」状態を、**1 体感バグ＝1 スライス**の着手順に落とす。

### 非目的（本正本では扱わない / 他へ委譲）

| 対象 | 委譲先 |
|:---|:---|
| Scene Store CRDT / Carry-Over 契約そのもの | [scene_store_sim_arena.md](scene_store_sim_arena.md) |
| CCU / cell interest | [spatial_interest.md](spatial_interest.md) |
| Drive Git / clone·fetch / VCS UI | [assets-vcs-draft.md](../0_doc/architecture/assets-vcs-draft.md) |
| social revoke / kick deny / cell 回復 | [improvement-plan-2026-11-18.md](../0_doc/evaluation/improvement-plan-2026-11-18.md) |
| rapier3d / AVBD 再導入 | 人承認・別決（guardrails） |
| Proprietary 検疫閾値 | OSS に置かない（licence-and-boundaries） |
| Dual-AST Kernel / ProtoFlux 本丸 | overview 目標・本線外 |

---

## 2. 現状ギャップ（器 done vs 体験）

| 領域 | 状態 | 体験への意味 |
|:---|:---|:---|
| Scene Store / Sim Arena / SoftLease / PendingLocalOps | **done** | 共同編集・楽観予測の**器** |
| TELEPORT → GPU snap | **後続**（Arena 面のみ） | 生成・Reparent の残像・ちぎれ |
| Depth Bucket / bitmask edge | Arena **非保証** | 親子編集・Enabled 切替の手触り |
| AvatarRig / RigidSolver | 境界 **done**・host tick **未配線** | 揺れもの・剛体が画面に出ない |
| Inspector（x00bc） | 数値表示まで | スクラブ → `…/edit` Commit は非保証 |
| XR レーザー | aim があれば**常時ビーム** | Primary=0 で消えない |
| `InteractionLaserConfig` | 変調＋stickiness はコードあり | Settings 未露出・`ShowInDesktop` 未 |
| Dash | CPU Canvas blit（仮） | ガラス風本番見た目は未 |
| Facet | `FacetId` / Pinned Exit のみ | グリッド装着・ユーザー追加は非保証 |
| 文字 | DirectNumber ＋ Dash 5×7 ASCII | MSDF / CJK / 文言カタログは未 |

**原則:** 新アルゴリズムより、**既存契約を Ring2→Ring3 / UI まで通す**。Phase 飛び越し禁止の精神は維持（品質本線の契約を壊さない）。

---

## 3. 推奨着手順（要約）

```text
L0  Primary レーザー表示ゲート（独立・即効）
F0  TELEPORT GPU → Depth Bucket → bitmask edge → PendingLocalOps 実操作接続
G0  ガラス風マテリアル（ブラーなし）＋公式 Dash を板経路へ寄せる布石
T1  Static MSDF（ASCII / 数字 / アイコン）
L1  Settings ↔ InteractionLaserConfig（Resonite LaserSettings 写像）
F1  Inspector Commit / SoftLease 見える化 / ドラッグ揮発↔Commit
Fc0–Fc1  FacetSlot ＋ UI Edit Mode（公式だけ並べ替え）
L2  XR ビームへ角度変調スムージング本配線
Fc2–Fc3  レイアウト永続 ＋ マウント API
T2 / E2-1  CJK オンデマンド atlas ＋ XR IME（依存は人承認）
U1  公式 UI データ駆動
F2  AvatarRig host 配線 / アバター見た目 A / 剛体は別決
Fc4–Fc5  Inventory Spawn Facet / Facet Anchors
U2  CustomUIX（誰でも編集できる中身）
G1  アクティブ板だけの任意 blur（実測後）
E1  ヒッチ根絶（スキニング / Async PSO / LOD builder）
Fc6  Settings 行 → Facet 化
```

**次の着手（固定）:** スライス発行時は **`x00ea` = L0（Primary レーザー表示ゲート）** を第一候補とする。並行で F0-1（TELEPORT GPU snap）を切ってよい（依存がほぼ直交）。

---

## 4. Wave 詳細

### 4.1 F — Feel wiring（編集体験の物理的正しさ）

| ID | 内容 | なぜ先か | 根拠・注意 |
|:---|:---|:---|:---|
| **F0-1** | **TELEPORT → GPU snap** | 生成・SetParent・ワープの残像が編集を壊す | [scene_store_sim_arena.md](scene_store_sim_arena.md) Ring3「TELEPORT GPU snap は後続」。Arena 寿命契約は既存 |
| **F0-2** | **Depth Bucket 階層伝播** | 親子編集のワールド座標 | Arena Step 3。アバター末端は Slot に載せない |
| **F0-3** | **bitmask edge Observer** | Enabled ON 初速リセット等 | Step 1。物理掴み SSoT は `holder_slot`（SoftLease と二重化しない） |
| **F0-4** | **PendingLocalOps を実操作へ** | x00e6 は器。Grab/編集経路へ接続 | Published 非汚染を崩さない |
| **F1-1** | Inspector スクラブ → SetField Commit | x00bc が明示的に保証していない穴 | SoftLease と整合 |
| **F1-2** | SoftLease の見える化 | 編集ロックの「誰が握っているか」 | 物理 Grab と別概念 |
| **F1-3** | ドラッグ揮発 ↔ Commit 1 回 | 密座標を CRDT に載せない契約の UI 側 | 既存 grab / edit レーン |
| **F2-1** | AvatarRig → host tick 配線 | XPBD が画面に出る | Slot 爆発禁止 |
| **F2-2** | アバター見た目 A（root 静的 GLB） | ボックス→人型 | [avatar-animation-ux.md](memo/avatar-animation-ux.md) 段 A |
| **F2-3** | 剛体ソルバ本体 | **人承認・別決**。当面既存 XPBD 接触可 | rapier/AVBD 未採択 |

### 4.2 L — Laser（Resonite 流）

参照: [Resonite Laser Settings](https://www.resonitemagazine.com/components/lasersettings/) · 正本 §6.2 · 実装 [`client/app/src/spatial_lens/laser.rs`](../../client/app/src/spatial_lens/laser.rs) · XR は [`wgpu_session.rs`](../../client/app/src/surface/xr/wgpu_session.rs)

| ID | 内容 | 備考 |
|:---|:---|:---|
| **L0** | **Primary 表示ゲート** | `primary < laser_show_primary_min`（既定 **0.1**）でビーム非描画。`≥ 0.1` で表示。Grab/UI 二値しきい値（現行 ≈0.7）とは **別パラメータ** |
| **L1** | Settings に Laser 項目を露出・永続化 | 下表写像 → `InteractionLaserConfig` |
| **L2** | XR ビーム向きに角度変調スムージング本配線 | 現状は Lens Desktop 寄り |
| **L3** | ベジェ描画・ShowInDesktop・Cursor 倍率 | x00bd 非保証の仕上げ |

**LaserSettings 写像**

| Resonite | Alchemy |
|:---|:---|
| SmoothSpeed | `laser_smooth_speed` |
| ModulateStartAngle | `modulate_start_angle_deg` |
| ModulateEndAngle | `modulate_end_angle_deg` |
| ModulateExponent | `modulate_exponent` |
| ModulateSpeedMultiplier | `modulate_speed_multiplier` |
| StickThreshold | `laser_stickiness`（改名可） |
| ShowInDesktop | `show_lasers_in_desktop_mode`（**追加**） |
| （Alchemy）表示しきい値 | `laser_show_primary_min`（既定 0.1）（**追加**） |

### 4.3 T — Text / 多言語（描画とカタログを分離）

正本: Lens アーキ §4.2 / §5 危険①③ · Expansion **E2-1**

| 層 | 対象 | 設計 | 現状 |
|:---|:---|:---|:---|
| **A** | ライブ数値 | **Zero-String / DirectNumber**（`format!` 禁止） | **実装済み**（x00bc） |
| **B / T1** | ASCII・数字・アイコン | Tier 1 Static MSDF Atlas → `MsdfGlyph` | 種別のみ・アトラス未 |
| **C / T2** | CJK・絵文字・ノード名・チャット | Tier 2 オンデマンド SDF + LRU（表示中のみ） | 未。`cosmic-text` / `etagere` は **人承認** |
| **文言カタログ** | 公式 UI 固定文言 | message id → ロケール解決 → **解決後 UTF-8 を MsdfGlyph へ** | client 正本なし（描画器の後でよい） |
| **入力** | Active Focus 1 枠 / XR IME | 危険③ · E2-1 | 未 |

**やらない:** 全漢字の事前ベイク。ライブ数値を String / カタログに載せる。

### 4.4 G — Glass（公式ダッシュ見た目）

添付コンセプト（半透明・縁ハイライト・カードレイアウト）を本番目標とする。現行 Dash は CPU Canvas blit（仮）。

| ID | 内容 | 注意 |
|:---|:---|:---|
| **G0** | 「ガラスっぽく見える」安価版：半透明＋縁 glow＋内側グラデ。**背景ブラーなし** | Lens 正本 §5 危険②。標準 α≈0.92 ダークスモーク |
| **G1** | アクティブ最前面板だけの任意 blur / downsample | VR 90Hz が保てるときだけ。本線初期にしない |

公式シェルは最終的に **1 パネル＝1 板＋`GpuUiPrim`**（Spatial Lens 系）へ寄せる。フルスクリーン grab-pass ブラーを第一本番にしない。

### 4.5 Fc — Facet 土台（UIX より先）

参照: [Resonite Facets](https://wiki.resonite.com/Facets) · 既存 [x000f](slices/x000f-dash-facet-minimal.md)

| ID | 内容 |
|:---|:---|
| **Fc0** | `FacetInstance` / `DashGrid` スロットモデル（個人・Room 非公開）。Pinned Exit 維持 |
| **Fc1** | UI Edit Mode ＋ 公式 Facet のグリッド装着・並べ替え・リサイズ |
| **Fc2** | レイアウト永続（ローカル → 将来 Vault） |
| **Fc3** | マウント API（公式プレースホルダ / 薄いスキーマ解釈） |
| **Fc4** | Inventory「Spawn Facet」→ 個人空間（中身は段階導入） |
| **Fc5** | Facet Anchors（Hand / Forearm / View）。userspace・自分だけ |
| **Fc6** | Settings 行 → Facet 化（六点 tear-off 相当） |

**契約:** UI 部品を空間 Slot にしない。Dash 変更は Room に載せない。Pinned は外し不可。

### 4.6 U — UIX（誰でも編集）

参照: [Resonite UIX](https://wiki.resonite.com/UIX) · Lens 正本 `CustomUIX` · 危険④（dirty 時のみ Flex）

| ID | 内容 |
|:---|:---|
| **U0** | 公式 Dash を板＋`GpuUiPrim` 経路へ（G0 と一体でも可） |
| **U1** | 公式 UI のデータ駆動（コードハードコードから剥がす） |
| **U2** | ユーザー CustomUIX（Scene Store 疎データで共同編集可能な中身） |
| **U3** | Facet／ワールド制御盤の本格結線（Dual-AST は別本線） |

**U2 より前に Resonite 流 Slot ツリー UIX を再発明しない。** Fc0–Fc3 の装着土台が先。

### 4.7 E — Expansion（並走・後段）

[memo/alchemy-expansion.md](memo/alchemy-expansion.md) のまま。

| ID | 内容 | 本正本での位置 |
|:---|:---|:---|
| E1-1 | SkeletonPosePage / Morph SoA + GPU skinning | F2-2 の後が自然 |
| E1-2 | Async PSO + PBR フォールバック | カスタムシェーダー投入前に器だけ先行可 |
| E1-3 | LOD / KTX2 低優先度ビルド | x0085 系と矛盾させない |
| E2-1 | MSDF + XR IME | **T2 と同一帯** |
| E2-2 | ローカル CAS Pin + LRU GC | 長時間運用。体験本線と直交し並走可 |

---

## 5. スライス発行ルール

1. **1 Wave 項目 ≈ 1 `slices/xNNNN-slug.md`**（必要なら F0-1 のようにサブ ID を slug に含める）。
2. 各スライスに **保証する / しない** を必ず書く。体感で何が直るか **1 文**を目標に含める。
3. **新規依存クレート**（例: `cosmic-text`, `etagere`, `taffy`, rapier）は guardrails どおり **人承認**。未承認で足さない。
4. `shared-sim` に OS スレッド・`Instant`・wgpu・ソケットを持ち込まない。
5. セキュリティ境界（Room Token / ManifestSeal / actor_id / AEAD）を弱めない。
6. 発行したら [README.md](README.md) 目次・本節「次の着手」・必要なら品質本線の「次の着手」一文を更新する。

### 推奨最初のスライス案

| 候補 ID | slug 案 | Wave | DoD 要約 |
|:---|:---|:---|:---|
| x00ea | `laser-primary-visibility` | L0 | Primary &lt; 0.1 で XR ビーム非描画。Grab 二値しきい値は不変 |
| x00eb | `teleport-gpu-snap` | F0-1 | `STATE_TELEPORT_THIS_TICK`（または同等）で補間スキップ／スナップ。残像回帰テスト |

---

## 6. 他本線との境界

| 混ぜない | 理由 |
|:---|:---|
| Drive Git / VCS 詳細パネル | 体験本線ではない（並行候補のまま） |
| 規模 CCU KPI | [spatial_interest.md](spatial_interest.md) のみ |
| 剛体ソルバ本実装 | 人承認・別決 |
| Spot-Check 統計 | Proprietary |
| 品質本線の契約改変を「見た目のため」に行うこと | 先祖返り・SSoT 並存を再発させる |

---

## 7. 関連リンク（外部）

- [Resonite Laser Settings（レゾマガ）](https://www.resonitemagazine.com/components/lasersettings/)
- [Resonite UIX](https://wiki.resonite.com/UIX)
- [Resonite Facets](https://wiki.resonite.com/Facets)

---

## 8. 改訂メモ

| 日付 | 内容 |
|:---|:---|
| 2026-11-28 | 初版。品質本線クローズ後の体験本線として F/L/T/G/Fc/U/E を固定。次着手候補 L0 / F0-1 |
