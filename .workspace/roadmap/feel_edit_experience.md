# Alchemy — 体感・編集体験本線（Feel / Edit）

* **種別**: **体験本線**の優先度・段階正本（品質本線枠クローズ後の「配線・操作感・公式 UI・拡張土台」）
* **前提**: 品質本線 [scene_store_sim_arena.md](scene_store_sim_arena.md) Phase 1–3 枠は [x00e9](slices/x00e9-rigid-solver-boundary.md) で **done / クローズ**（器はある。体感経路の多くは未配線）
* **規模本線**: [spatial_interest.md](spatial_interest.md)（本ファイルと混同しない）
* **UI 第一原理**: [alchemy_zero_base_spatial_ui_lens_uix_architecture.md](alchemy_zero_base_spatial_ui_lens_uix_architecture.md)（Spatial Lens / Uber-SDF / Zero-String / CustomUIX）
* **拡張ギャップ（ヒッチ・IME）**: [memo/alchemy-expansion.md](memo/alchemy-expansion.md)（**E1 / E2** — 本線番号と非共有）
* **アバター見た目・ダンス**: [memo/avatar-animation-ux.md](memo/avatar-animation-ux.md)（本線番号と非共有）
* **既存 Facet 最小**: [slices/x000f-dash-facet-minimal.md](slices/x000f-dash-facet-minimal.md)（語彙のみ。並べ替え・ユーザー追加は非保証）
* **空きスライス ID**: **`x00ef`** 以降（発行時は [README.md](README.md) と当該 `slices/` を更新）

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
| Scene Store / Sim Arena / SoftLease / PendingLocalOps | **done**（Grab 配線は [x00f1](slices/x00f1-pending-local-ops-wire.md)） | 共同編集・楽観予測の器＋Grab ミラー |
| TELEPORT → GPU snap | **done**（[x00eb](slices/x00eb-teleport-gpu-snap.md)） | 生成・Reparent の残像・ちぎれを Curr へ即スナップ |
| Depth Bucket / bitmask edge | **done**（[x00ef](slices/x00ef-depth-bucket-hierarchy.md) / [x00f0](slices/x00f0-bitmask-edge-observer.md)） | 親子追従・Enabled ON 初速リセット |
| AvatarRig / RigidSolver | 境界 **done**・host tick **未配線** | 揺れもの・剛体が画面に出ない |
| Inspector（x00bc / [x00f2](slices/x00f2-inspector-setfield-commit.md)） | Desktop POS スクラブ → SetField Commit | SoftLease 見える化は → F1-2；XR スクラブは非保証 |
| XR レーザー | **L0+L1 done**（Primary ゲート＋Settings 永続） | ShowInDesktop 可視化ビームは → L3；XR smooth → L2 |
| `InteractionLaserConfig` | Settings 露出＋`laser.json` 永続（[x00ee](slices/x00ee-laser-settings-l1.md)） | Desktop ビーム描画本体（→ L3） |
| Dash | CPU Canvas blit（仮）＋G0 色トークン共有 | 板経路の角丸・glow は Lens 済み；Dash 全板移行は U0 |
| Facet | `FacetId` / Pinned Exit のみ | グリッド装着・ユーザー追加は非保証 |
| 文字 | DirectNumber ＋ Lens Static MSDF（[x00ed](slices/x00ed-static-msdf-t1.md)）＋ Dash 5×7 | CJK / 文言カタログは未（→ T2） |

**原則:** 新アルゴリズムより、**既存契約を Ring2→Ring3 / UI まで通す**。Phase 飛び越し禁止の精神は維持（品質本線の契約を壊さない）。

---

## 3. 推奨着手順（要約）

```text
L0  Primary レーザー表示ゲート（独立・即効）
F0  TELEPORT GPU → Depth Bucket → bitmask edge → PendingLocalOps 実操作接続
G0  軽量ガラス語彙（シェル＋Card/Control・ブラーなし）＋板経路布石
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

**次の着手:** F1-1 **done**（[x00f2](slices/x00f2-inspector-setfield-commit.md)）。次候補 **F1-2** SoftLease 見える化（空き ID **`x00f3`**）。F0 / L1 / T1 / G0 も done。

---

## 4. Wave 詳細

### 4.1 F — Feel wiring（編集体験の物理的正しさ）

| ID | 内容 | なぜ先か | 根拠・注意 |
|:---|:---|:---|:---|
| **F0-1** | **TELEPORT → GPU snap**（[x00eb](slices/x00eb-teleport-gpu-snap.md) **done**） | SetParent・ワープの残像が編集を壊す | Arena → sheet `FLAG_TELEPORT` → Epoch lerp/WGSL スナップ。**新規生成**は pack ABA。ワイヤ Frame bit / PendingLocalOps 自動供給は非保証 |
| **F0-2** | **Depth Bucket 階層伝播**（[x00ef](slices/x00ef-depth-bucket-hierarchy.md) **done**） | 親子編集のワールド座標 | host Step 3：**depth 順一巡**（祖先確定→dirty invert→compose）。子も Grab/動力学可。伝播は pos/scale のみ。Reparent 契約 A |
| **F0-3** | **bitmask edge Observer**（[x00f0](slices/x00f0-bitmask-edge-observer.md) **done**） | Enabled ON 初速リセット等 | Step 1。物理掴み SSoT は `holder_slot`（SoftLease と二重化しない） |
| **F0-4** | **PendingLocalOps を実操作へ**（[x00f1](slices/x00f1-pending-local-ops-wire.md) **done**） | x00e6 は器。Grab 経路へ接続 | Published 非汚染を崩さない。Inspector Commit は → F1 |
| **F1-1** | Inspector スクラブ → SetField Commit（[x00f2](slices/x00f2-inspector-setfield-commit.md) **done**） | x00bc が明示的に保証していない穴 | SoftLease ゲート流用。Save pose 永続は非保証 |
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
| **L1** | Settings に Laser 項目を露出・永続化 | [x00ee](slices/x00ee-laser-settings-l1.md) **done**。下表写像 → `InteractionLaserConfig` / `laser.json` |
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
| **B / T1** | ASCII・数字・アイコン | Tier 1 Static MSDF Atlas → `MsdfGlyph` | **done**（[x00ed](slices/x00ed-static-msdf-t1.md)；疑似 MSDF R=G=B） |
| **C / T2** | CJK・絵文字・ノード名・チャット | Tier 2 オンデマンド SDF + LRU（表示中のみ） | 未。`cosmic-text` / `etagere` は **人承認** |
| **文言カタログ** | 公式 UI 固定文言 | message id → ロケール解決 → **解決後 UTF-8 を MsdfGlyph へ** | client 正本なし（描画器の後でよい） |
| **入力** | Active Focus 1 枠 / XR IME | 危険③ · E2-1 | 未 |

**やらない:** 全漢字の事前ベイク。ライブ数値を String / カタログに載せる。

### 4.4 G — Glass（公式ダッシュ見た目）

添付コンセプト（半透明・縁ハイライト・カードレイアウト）を本番目標とする。現行 Dash は CPU Canvas blit（仮）。最終形は **1 パネル＝1 板＋`GpuUiPrim`**（Spatial Lens / Uber-SDF）。フルスクリーン grab-pass ブラーを第一本番にしない。

参照: Lens 正本 §5 危険② · [`GpuUiPrim` / `rounded_box_payload`](../../client/app/src/spatial_lens/prim.rs) · [`uber_sdf.wgsl`](../../client/app/src/spatial_lens/shaders/uber_sdf.wgsl) · 現行色定数 [`dash.rs`](../../client/app/src/system_ui/dash.rs)

| ID | 内容 | 注意 |
|:---|:---|:---|
| **G0** | 軽量ガラス語彙の契約＋実装：半透明＋縁 glow＋内側グラデ。**背景ブラーなし**。シェル板だけでなく Card / Chrome / Control / Field も同じ語彙 | [x00ec](slices/x00ec-glass-material-g0.md) **done**。標準 α≈0.92。トークン・payload・シェーダ式は本節が正本 |
| **G1** | アクティブ最前面板だけの任意 blur / downsample | VR 90Hz が保てるときだけ。本線初期にしない |

#### G0 保証する / しない

| 保証する | 保証しない |
|:---|:---|
| 役割トークン（下表）と RoundedBox payload 意味（角丸・縁・glow） | 背景ブラー / grab-pass / downsample glass（→ G1） |
| Uber-SDF 1 パスでの内側縦グラデ＋縁 soft glow | Dash 全コンテンツの板移行完了（→ U0） |
| Desktop / XR / 将来 Web で **同一トークン＋同一 WGSL** | DOM/CSS `backdrop-filter`・プラットフォーム別マテリアル |
| LOD2 板色を `ShellFill` 系に揃えること | MSDF / CJK（→ T）・Facet 並べ替え（→ Fc） |

#### 役割トークン（既存 `dash.rs` 色を基準に正規化）

実装時は 1 箇所の Rust 定数（例: `spatial_lens` または薄い `system_ui/glass.rs`）へ落とす。散在リテラルを増やさない。

| 役割 | 用途 | 基準色（現行） | α / 備考 |
|:---|:---|:---|:---|
| `ShellFill` | パネル外枠板 | `BG` `(0.06, 0.07, 0.09)` | **0.92**（危険②標準） |
| `ShellBorder` | 外枠縁ハイライト | 明度を上げたニュートラル（新規定数可） | 低 α。glow のベース色 |
| `ChromeFill` | 上下バー | `BAR` `(0.10, 0.11, 0.14)` | ≤0.96、ほぼ不透明寄り |
| `CardFill` | カード／パネル内面 | ≈`(0.11–0.13, …)` 系 | Shell よりわずかに明るく・α 高め可 |
| `ControlIdle` | ボタン／タブオフ | `BTN` / `CLOSE_BTN` | Depth Write 対象（不透明寄り） |
| `ControlHot` | ホバー | `ICON_HOT` | |
| `FieldIdle` / `FieldFocus` | 入力欄 | `FIELD` / `FIELD_FOCUS` | |
| `Accent` / `Danger` / `Text` / `Muted` | 意味色・文字 | 現行どおり | 文字は α=1 |

コンテンツ固有カード色（`OFFICIAL_CARD` / `FED_CARD` / `CUSTOM_CARD` 等）は **Accent 系オーバーレイ**として残す。ガラス基材（α・角丸・縁・glow）は上表の共通トークンに乗せる。

#### RoundedBox payload（64B 契約を壊さない）

既存 `rounded_box_payload(corner_radius, border_thickness, glow)`:

```text
payload = [corner_radius, border_thickness, glow, 0]
```

| フィールド | 意味 | 標準値 |
|:---|:---|:---|
| `corner_radius` | px | Shell 12–16 / Card 8 / Control 6 |
| `border_thickness` | px | Shell 1.5 / Card 1 / Control 1 |
| `glow` | 縁外側の安価 soft edge 強度（0=OFF） | 通常 0–0.25。**アクティブ最前面板の Shell のみ** ≤1.0 |
| `payload.w` | 予約 | **当面 0**。内側縦グラデは UV 固定式（パラメータを増やさない＝軽さ優先） |

実装（[x00ec](slices/x00ec-glass-material-g0.md)）: `payload.z` glow を `uber_sdf.wgsl` が参照。Shell prim は **`shell_rect_inset`（inset = `border_t + 6×glow` の active 到達幅で固定）**し、clip＝板全面でも外側 soft glow が落ちない。hover で glow 強度だけ変わり、シェル枠はジャンプしない。

**active glow:** レーザー／ホバーが乗った最前面板の Shell のみ ≤1.0（`hovered_binding != 0`）。単一 Lens が visible なだけでは idle（0.20）。選択行はシェル glow を上げない。

#### シェーダ見た目（ブラーなし・1 パス）

`fs_uber_sdf` RoundedBox 分岐の仕様:

1. 既存 SDF fill + border
2. **内側グラデ**: prim 局所 `local_uv.y` で fill RGB を ±数 % だけ混ぜる（追加テクスチャサンプルなし）
3. **縁 glow**: `glow > 0` のとき `smoothstep` で外側に `ShellBorder` / border 色を薄く足すだけ（CPU 側インセット前提）
4. **LOD2**（`lod_level >= 2`）: 中身 prim を描かず板色のみ。WGSL リテラルは `glass::LOD2_SHELL` / `lod2_wgsl_vec4_literal()` と一致させる（gpu テストで固定）

**禁止（G0）:** フルスクリーン／パネル単位の背景ブラー、2 パス以上の glass、プラットフォーム分岐マテリアル、CPU Canvas 側だけの別ガラス表現を本番とすること。

#### プラットフォーム統一（Desktop / XR / Web）

| 規則 | 内容 |
|:---|:---|
| **単一 SSoT** | トークン表は本節。実装定数は 1 モジュール。Web 到着時も同じ定数＋同じ WGSL |
| **板経路が本保証** | 角丸・glow・内側グラデは Uber-SDF 板経路でのみ本保証 |
| **CPU Dash 過渡** | U0 完了まで Desktop/XR の CPU Canvas は **同じ RGB/α を写す**（見た目近似）。角丸・glow は非保証 |
| **Web** | surface 未実装でもマテリアル分岐を作らない。CSS glass / `backdrop-filter` は採用しない |

```text
DashGlassTokens → RoundedBox pack → uber_sdf.wgsl
                      ↓                ↓
              Desktop / XR（今）    Web（将来・同一）
```

#### G0 / U0 / G1 境界

| ID | 保証 | 非保証 |
|:---|:---|:---|
| **G0** | トークン＋RoundedBox 見た目（グラデ・glow・α）の契約と実装 | Dash 全板移行完了、blur |
| **U0** | 公式 Dash を 1 板＋`GpuUiPrim` へ寄せる配線（G0 と一体でも可） | Facet 並べ替え・CustomUIX |
| **G1** | アクティブ 1 枚の任意 blur（実測後） | 本線初期 |

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
| x00eb | `teleport-gpu-snap` | F0-1 | `STATE_TELEPORT_THIS_TICK` → sheet `FLAG_TELEPORT` → GPU snap。残像回帰テスト |

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
| 2026-11-28 | x00ea L0 発行・done。次着手候補 x00eb F0-1 |
| 2026-11-28 | x00eb F0-1 TELEPORT GPU snap done。次着手候補 x00ec F0-2 |
| 2026-11-28 | G0 軽量ガラス・マテリアル契約を §4.4 に固定（トークン・payload・シェーダ・Win/Web 統一・G0/U0/G1）。実装スライスは未発行 |
| 2026-11-28 | [x00ec](slices/x00ec-glass-material-g0.md) G0 done（glass トークン・Uber-SDF glow/グラデ・Dash 過渡色）。次候補 x00ed F0-2 |
| 2026-11-28 | G0 レビュー修正: Shell `glow_pad` インセット、active＝hover のみ、LOD2 リテラル同期、GlassColor packed 導出、正本陳腐記述削除 |
| 2026-11-28 | G0: inset=`border_t+6×glow`（active 固定）・`shell_glow_extent_wgsl_expr` 同期・hover でシェル枠非ジャンプ |
| 2026-11-28 | [x00ed](slices/x00ed-static-msdf-t1.md) T1 Static MSDF done（512² atlas・MsdfGlyph・DirectNumber アトラス桁）。次候補 x00ee F0-2 |
| 2026-11-28 | [x00ee](slices/x00ee-laser-settings-l1.md) L1 Settings ↔ InteractionLaserConfig 着手。次候補 x00ef F0-2 |
| 2026-11-28 | [x00ee](slices/x00ee-laser-settings-l1.md) L1 done（Settings Laser 節・laser.json・Desktop/XR cfg 反映）。次候補 x00ef F0-2 |
| 2026-11-28 | x00ee レビュー追随: 体感文を XR=Primary / Desktop=smooth·stick に揃える・Laser レイアウト定数一本化・prefs 非同期 coalesce・非有限 clamp |
| 2026-11-28 | x00ee prefs: spawn 失敗 sync drain・`flush_blocking`（exit）・FLUSHING 中 kick 抑止・join を lock 外へ |
| 2026-11-28 | [x00ef](slices/x00ef-depth-bucket-hierarchy.md) F0-2 Depth Bucket done（locals・契約 A・Step 3）。次 [x00f0](slices/x00f0-bitmask-edge-observer.md) |
| 2026-11-28 | [x00f0](slices/x00f0-bitmask-edge-observer.md) F0-3 bitmask edge done。[x00f1](slices/x00f1-pending-local-ops-wire.md) F0-4 Grab↔PendingLocalOps done。次候補 F1-1 |
| 2026-11-28 | [x00f2](slices/x00f2-inspector-setfield-commit.md) F1-1 Inspector POS スクラブ → SetField Commit done。次候補 F1-2 |
