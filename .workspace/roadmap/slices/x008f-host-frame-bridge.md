# x008f — host 60Hz FrameBridge 配線（Rapier 影並走）

> **ID:** `x008f` · **slug:** `host-frame-bridge` · **status:** `done` · **area:** `[host, sim]` · **depends_on:** `[x008e, x0006]`  
> **優先度:** Zero-Base Spatial 本線（x008e の「次」）· improvement **P2-6**（tick 切り出し）を同帯で吸収可

目標: host の **60Hz Rapier ループに `FrameBridge::tick_mirror_publish` を本番配線**し、`EpochSnapshotPair`（Prev/Curr）を **変化時（TRS dirty / membership）** に公開する。ワイヤ Frame / Pose の権威は **引き続き Rapier**。影経路では **XPBD / LBVH / contact を回さない**（kinematic mirror: `inv_mass=0`・`DYNAMIC` なし）。

根拠: [x008e](x008e-zero-base-spatial-lifecycle.md)「やらない／次」先頭 · [alchemy_zero_base_spatial_lifecycle_architecture.md](../alchemy_zero_base_spatial_lifecycle_architecture.md) · [improvement-plan-2026-11-05.md](../../0_doc/evaluation/improvement-plan-2026-11-05.md) **P2-6**

---

## 保証する / しない

| 保証する | 保証しない |
|:---|:---|
| 各 physics tick で `FrameBridge::tick_mirror_publish`（失敗はログ・ループ継続）。**XPBD / LBVH / contact は回さない** | XPBD 結果を Zenoh Frame / snapshot に載せる；影経路での空間クエリ本番 |
| Rapier の **dirty TRS のみ** tape へ mirror（`inv_mass=0`）→ 変化時のみ Prev/Curr publish | 毎 tick 全行 patch / 無変化時の sheet 二重 clone |
| **reconcile が tape を変えたときだけ `force_publish`**（alloc/tombstone 成功。quarantine 清掃のみでは force しない） | ensure 失敗の毎 tick force（`ensure_skipped` で退避） |
| user_root 影 collider = Rapier と同型 `SPHERE r=0.3` | 将来カプセル歩行の先行導入 |
| `entity_id → NodeHandle` の reconcile（mesh / box / user_root；spawn・despawn 追随） | floor 等 Manifest 外静的体の tape 化 |
| 単体テスト: 移動体 Prev≠Curr；spawn/despawn が **published Curr** に反映；idle は publish スキップ | SceneGpu 補間 bind・client `epoch_view` 本番購読 |
| host `main` から tick 本体を `epoch_bridge` に閉じる（P2-6 の一部） | pose ダブルバッファの wire 経路置換 → **[x0095](x0095-wire-pose-double-buffer.md)**（本 Slice 時点では未） |

---

## 設計（要点）

```
physics.apply_* / physics.step()     ← Rapier（wire 権威）
        │
        ▼
EpochBridgeHost::tick_after_rapier
  if membership_changed → reconcile → tape_mutated?
  queue dirty SetTransform patches only
  FrameBridge::tick_mirror_publish(force = tape_mutated)
    → no XPBD; publish+commit iff patches|reparent|force
        │
        ▼
既存 30Hz pose sample / network_worker   ← 変更なし（Rapier 由来）
```

- Shadow 行の readiness: `ALLOCATED | TRANSFORM | COLLIDER`（mesh は `MESH` も）。**`DYNAMIC` を立てない**。
- `tombstone` 失敗時は `entity_rows` に handle を戻し、次 tick で再試行（map だけ失って orphan 行を残さない）。
- 診断カウンタ: `barrier_ticks`（barrier 呼び出し）と `publishes`（実 ArcSwap 更新）を分離。
- Grab / Intent は現行 Rapier 経路のまま。tape 側 Grab ビットは本 Slice では触らない。

---

## Phase

| Phase | 内容 | 状態 |
|:---|:---|:---|
| 0 | Slice + roadmap 目次 | **done** |
| 1 | `host-server` `epoch_bridge` モジュール + 単体テスト | **done** |
| 2 | `main` 60Hz ループ配線 + spawn/despawn reconcile | **done** |
| 3 | docs / improvement P2-6 言及・完了条件緑 | **done** |

---

## 完了条件

1. `cargo test -p host-server`（および binary 内 `epoch_bridge` テスト）緑
2. 60Hz ループ内で `FrameBridge::tick_mirror_publish` が実行される（コード経路 + 単体）
3. 移動体で publish 面の Prev/Curr 位置が一致しない回帰テスト
4. spawn/despawn が **published** Curr に反映される回帰テスト（`entity_rows` のみでは不足）
5. wire Frame の pose 取得経路が Rapier のままであること（差分レビュー）
6. Rapier / SlotGraph を削除していないこと

## やらない（本 Slice）／次

- XPBD を wire 権威にする / Rapier 削除 → **[x0093](x0093-xpbd-wire-authority.md)**
- SceneGpu 補間コンピュート bind → **[x0090](x0090-client-epoch-scenegpu-bind.md)**
- client が EpochSnapshotPair を消費する経路 → **[x0090](x0090-client-epoch-scenegpu-bind.md)**（wire Frame からローカル再建）
- Loro CRDT / Zenoh Grab ストリーム
- UI Lens 本仕様

## 関連

- [x008e-zero-base-spatial-lifecycle.md](x008e-zero-base-spatial-lifecycle.md)
- [x0006-rapier-finite-tick.md](x0006-rapier-finite-tick.md)
- [alchemy_zero_base_spatial_lifecycle_architecture.md](../alchemy_zero_base_spatial_lifecycle_architecture.md)

← [目次](../README.md)
