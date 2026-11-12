# x0090 — client Epoch 購読 + SceneGpu 最小 bind

> **ID:** `x0090` · **slug:** `client-epoch-scenegpu-bind` · **status:** `done` · **area:** `[client, sim]` · **depends_on:** `[x008e, x008f, x0023]`  
> **優先度:** Zero-Base Spatial 本線（x008f の「次」）

目標: 検証済み `RenderFrame` 適用後に client ローカル `EpochSnapshotPair` へ kinematic mirror し、`SceneGpu` が Prev/Curr を storage buffer へ整列パックして interpolate compute を dispatch する。描画 UBO / wire 権威は **引き続き `EntityPose`（WorldView）**。

根拠: [x008f](x008f-host-frame-bridge.md)「やらない／次」· [alchemy_zero_base_spatial_lifecycle_architecture.md](../alchemy_zero_base_spatial_lifecycle_architecture.md) Step 5

---

## 保証する / しない

| 保証する | 保証しない |
|:---|:---|
| Applied Frame が `EpochViewState.snapshots` を更新（Prev＝直前 Curr、Curr＝新面） | host プロセスの ArcSwap を跨プロセス共有 |
| X-Ray が published Prev/Curr の実 pose を読める | Hierarchy UI 本配線・色の tape 化 |
| 表示フレームあたり1回 `sync_epoch_gpu`（pack → upload → interpolate；`draw_world` 外） | `out` を vertex 描画に使う（次 Slice） |
| kinematic mirror（`inv_mass=0`・`DYNAMIC` なし） | XPBD / Rapier 置換・wire Frame 変更 |
| mesh/box 描画は `EntityPose` のまま | topology 親子（本 Slice は flat） |

---

## 設計（要点）

```
verify_frame + tick gate
        │
        ├─► WorldView EntityPose          ← draw 権威
        │
        └─► EpochFrameMirror.ingest
              ensure/tombstone + SetTransform
              publish(prev=old curr, curr=new)
                    │
                    ▼
              EpochSnapshotPair (ArcSwap)
                    │
         ┌──────────┴──────────┐
         ▼                     ▼
      Tape X-Ray        SceneGpu sync_epoch_gpu
                        aligned pack → upload → compute
                        (out unused by draw)
```

- host と client は別プロセス → wire Frame からローカル再建。
- alpha: 最後の mirror publish と **wire Frame 30Hz**（物理 60Hz / 配信 30Hz · x0006）から clamp。Rapier tick には追従しない。
- `SceneGpu::sync_epoch_gpu` は **表示フレームあたり1回**（Desktop の draw 前 / XR は eye loop 前）。`draw_world` 内では呼ばない。
- pack は Prev TOMBSTONE / generation 不一致時 Curr を Prev にコピー（freelist ABA）。
- topology は触らない（X-Ray depth=0）。

---

## Phase

| Phase | 内容 | 状態 |
|:---|:---|:---|
| 0 | Slice + roadmap 目次 | **done** |
| 1 | `EpochFrameMirror` + `apply_frame` 配線 + 単体 | **done** |
| 2 | `SceneGpu` EpochGpuBuffers / compute bind | **done** |
| 3 | docs（x008e/x008f 次）完了条件緑 | **done** |

---

## 完了条件

1. `cargo test -p client`（mirror / pack 回帰）緑
2. Applied Frame 後に `load_curr()` が当該 entity 位置を持つ
3. snapshot replace / remove が published Curr から旧行を消す
4. 表示フレームあたり1回 `sync_epoch_gpu`（XR 二重 dispatch なし）
5. mesh/box 描画が `EntityPose` 由来のまま（差分レビュー）
6. host wire / Rapier 権威を変えていないこと

## やらない（本 Slice）／次

- `out` バッファを SceneGpu vertex / UBO に接続 → **[x0091](x0091-epoch-out-scenegpu-draw.md)** **done**（フル active pack；display TRS SSoT・Curr 単独権威は撤廃）
- カメラ／loco eye の display 化 → **[x0092](x0092-eye-display-trs.md)** **done**
- XPBD を wire 権威にする / Rapier 削除
- Loro CRDT / Zenoh Grab ストリーム
- UI Lens 本仕様
- wire pose ダブルバッファ置換（improvement P2-6 残り）
- Frame 受理時の world ロック中 TRS 収集のさらなる短縮（現状は KinematicPose のみ）

## 関連

- [x008e-zero-base-spatial-lifecycle.md](x008e-zero-base-spatial-lifecycle.md)
- [x008f-host-frame-bridge.md](x008f-host-frame-bridge.md)
- [x0023-p2-session-frame.md](x0023-p2-session-frame.md)

← [目次](../README.md)
