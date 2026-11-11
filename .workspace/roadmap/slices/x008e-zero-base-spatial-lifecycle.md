# x008e — Zero-Base Spatial Lifecycle（Tape / Topology / LBVH / XPBD / Bridge）

> **ID:** `x008e` · **slug:** `zero-base-spatial-lifecycle` · **status:** `done` · **area:** `[sim, client, host]` · **depends_on:** `[x0030, x0056]`  
> **正本仕様:** [alchemy_zero_base_spatial_lifecycle_architecture.md](../alchemy_zero_base_spatial_lifecycle_architecture.md)

目標: VR 共同編集向けに、外部物理エンジン二重状態を廃し、`EpochSheet`（MVCC SoA）＋ Flat Topology ＋ Morton LBVH ＋ 統合 XPBD ＋フレーム境界 Bridge を `alchemy-shared-sim` に実装する。Ring3 向けに `arc-swap` スナップショットと client `epoch_view`（補間 WGSL / Hierarchy / X-Ray）を接続する。

---

## 保証する / しない

| 保証する | 保証しない |
|:---|:---|
| `row_id` 不変・generation ABA・Hierarchical bitset・`is_finite` ゲート | Rapier の削除・本番 tick 置換 |
| Depth / Euler Baker・親→子伝播（間接配列のみ再構築） | 任意回転の完全な階層クォータニオン合成（並進＋scale 優先） |
| 30bit Morton + radix・Flat LBVH・Raycast・狭相（Sphere/Capsule/Box/Ray）・カプセル押し出し | StaticSDF 生成（Atom 参照スタブのみ）・凹メッシュ GJK/EPA |
| Prev/Curr 契約（publish は step 前 Prev＋step 後 Curr → その後 commit）・substep Verlet（`sub_prev`＝predict 直前） | Formula VM / 全身 IK UX |
| Grab / 距離 / soft compliance / sleeping・dirty・Patch Merge（物理座標 preserve） | Loro CRDT Commit・Zenoh 揮発ストリーム本番配線 |
| `arc-swap` Epoch 公開（**人承認済** MIT OR Apache-2.0）・GPU 行の **row_id 整列パック** | SceneGpu への補間コンピュート本配線・GPU Readback |
| Settings Tape X-Ray（`ensure_xray_row` + 読取分離）・`euler_order` 仮想 Hierarchy API | Spatial Lens UI 本仕様（別ドキュメント） |

---

## 実装スライス（実施単位）

仕様 Step 1–5 を細分化した実施単位。本 Slice で **shared-sim + client フックまで done**。

### P0 — 足場

| ID | 内容 | 状態 |
|:---|:---|:---|
| S-00 | `bytemuck`（workspace 既存）+ **`arc-swap` 人承認** + `tape`/`topology`/`spatial`/`xpbd`/`bridge` モジュール | **done** |

### P1 — `shared-sim::tape`（Step 1）

| ID | 内容 | 状態 |
|:---|:---|:---|
| S-01 | `NodeHandle` + `ReadinessMask` | **done** |
| S-02 | Pod 行型（`ColliderShape` / `WorldTransformf64` / `XpbdConstraintRow`） | **done** |
| S-03 | `EpochSheet` SoA・alloc / tombstone・row_id 不変 | **done** |
| S-04 | Hierarchical bitset 走査 | **done** |
| S-05 | `is_finite` サニタイズ | **done** |
| S-06 | `PendingPatch` + `apply_patches`（物理座標 preserve） | **done** |

### P2 — `shared-sim::topology`（Step 2）

| ID | 内容 | 状態 |
|:---|:---|:---|
| S-07 | `FlatTopology` + `parent_of` 契約 | **done** |
| S-08 | Depth Bucket Baker | **done** |
| S-09 | Euler Tour Baker | **done** |
| S-10 | 親→子 Transform 伝播 | **done** |
| S-11 | Constraint color batch 骨格 → XPBD で本実装 | **done** |

### P3 — `shared-sim::spatial`（Step 3 / Phase 1 物理）

| ID | 内容 | 状態 |
|:---|:---|:---|
| S-12 | 30bit Morton | **done** |
| S-13 | `(morton, row_id)` radix（key 一本化） | **done** |
| S-14 | Flat LBVH | **done** |
| S-15 | 狭相プリミティブ + StaticSDF stub | **done** |
| S-16 | Raycast（COLLIDER ビット） | **done** |
| S-17 | カプセル押し出し | **done** |

### P4 — `shared-sim::xpbd`（Step 4 / Phase 2 物理）

| ID | 内容 | 状態 |
|:---|:---|:---|
| S-18 | Prev/Curr + 慣性予測（substep `sub_prev`） | **done** |
| S-19 | 距離拘束（ハード） | **done** |
| S-20 | グラフ彩色 | **done** |
| S-21 | Grab（`GRABBED` / `node_b=MAX`） | **done** |
| S-22 | 接触（LBVH ペア・substep 再構築） | **done** |
| S-23 | Soft / Spring（compliance） | **done** |
| S-24 | Sleeping + dirty + デルタ API | **done** |

### P5 — Bridge（Ring 境界）

| ID | 内容 | 状態 |
|:---|:---|:---|
| S-25 | Patch Merge バリア・reparent 失敗ロールバック・dirty は publish Arc 消費 | **done** |
| S-26 | `EpochSnapshotPair`（arc-swap）・publish→clear_dirty→commit | **done** |

### P6 — client 接続（Step 5 最小）

| ID | 内容 | 状態 |
|:---|:---|:---|
| S-27 | wgpu 補間 WGSL + 整列パック API（SceneGpu 本配線は未） | **done**（API） |
| S-28 | `euler_order` 仮想 Hierarchy（window 走査） | **done**（API） |
| S-29 | Bitmask / Tape X-Ray（Settings + ensure 分離） | **done** |

---

## 主要契約（レビュー固定）

1. **Prev/Curr**: tick 開始面＝Prev、step 後＝Curr を publish。その後 `commit_prev_from_curr`（補間用と速度用は同一二面・別バッファにしない）。
2. **GPU パック**: `aligned_row_ids` → `pack_aligned_gpu_faces`（面間で同じ `row_id` 列）。独立 dirty パック禁止。
3. **ABA / append**: generation 不一致、または Prev TOMBSTONE かつ Curr live なら Curr→Prev コピー。Prev 伸長は `ensure_len_append`（freelist 不使用）。
4. **substep Verlet**: `sub_prev` は predict **直前**の Curr。solve 後に Curr で上書きしない。
5. **dirty**: publish 直後に live `curr` を `clear_dirty`。消費は published Arc 側。
6. **依存**: `arc-swap` 直接依存は人承認済み（`client/Cargo.toml` に注記）。

---

## 配置

| パス | 役割 |
|:---|:---|
| [`client/shared-sim/src/tape.rs`](../../../client/shared-sim/src/tape.rs) | Epoch SoA / Mask / Patch |
| [`client/shared-sim/src/topology.rs`](../../../client/shared-sim/src/topology.rs) | FlatTopology Baker / 伝播 |
| [`client/shared-sim/src/spatial.rs`](../../../client/shared-sim/src/spatial.rs) | Morton / LBVH / Ray / Extrude |
| [`client/shared-sim/src/xpbd.rs`](../../../client/shared-sim/src/xpbd.rs) | Native XPBD |
| [`client/shared-sim/src/bridge.rs`](../../../client/shared-sim/src/bridge.rs) | Frame barrier + ArcSwap |
| [`client/app/src/epoch_view.rs`](../../../client/app/src/epoch_view.rs) | Ring3 補間 / Hierarchy / X-Ray |

---

## 完了条件

1. `cargo test -p alchemy-shared-sim` 緑（Tape〜Bridge・XPBD 契約テスト含む）
2. client `epoch_view` 単体テスト緑
3. Prev/Curr publish が同一面にならないこと（回帰テスト）
4. multi-substep 落下が 1-substep と同一オーダー（過大／過少積分しない）
5. `arc-swap` 人承認が記録されていること

## やらない（本 Slice）／次

- host 60Hz への `FrameBridge::tick` 本番配線
- Rapier / 既存 `SlotGraph` の置換・削除
- SceneGpu への補間コンピュート bind（シェーダ／バッファ API のみ先行）
- Loro CRDT / Zenoh 揮発 Grab ストリーム
- Formula VM・全身 IK・StaticSDF asset-compiler
- UI Lens 本仕様 → [alchemy_zero_base_spatial_ui_lens_uix_architecture.md](../alchemy_zero_base_spatial_ui_lens_uix_architecture.md)

## 関連

- [alchemy_zero_base_spatial_lifecycle_architecture.md](../alchemy_zero_base_spatial_lifecycle_architecture.md)
- [x0030-shared-sim-entity-kind.md](x0030-shared-sim-entity-kind.md)
- [x0056-shared-sim-held-slot.md](x0056-shared-sim-held-slot.md)
- [x0006-rapier-finite-tick.md](x0006-rapier-finite-tick.md)（現行 Rapier 経路・置換対象）

← [目次](../README.md)
