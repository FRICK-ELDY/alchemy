# x0093 — XPBD wire 権威へ収束（Rapier 影並走廃止）

> **ID:** `x0093` · **slug:** `xpbd-wire-authority` · **status:** `done` · **area:** `[host, sim]` · **depends_on:** `[x008f, x0092]`  
> **優先度:** Zero-Base Spatial 本線（x008f 影並走の収束）

目標: host 60Hz の wire Frame TRS 権威を Rapier から **Native XPBD（EpochSheet）** へ移す。Intent / Grab / loco / spawn・despawn を tape 一本化し、`physics.step`（Rapier）と kinematic mirror（`tick_mirror_publish`）を廃止する。

根拠: [x008f](x008f-host-frame-bridge.md)「やらない／次」· [alchemy_zero_base_spatial_lifecycle_architecture.md](../alchemy_zero_base_spatial_lifecycle_architecture.md) · roadmap「現在の次」

---

## 保証する / しない

| 保証する | 保証しない |
|:---|:---|
| host wire Frame の TRS 権威は Epoch/XPBD | client が host ArcSwap を共有する |
| coin / grabbable box は `DYNAMIC` + `inv_mass>0`；床は地面平面（y=0）＋デモ重力 `-2.2` | Rapier 数値の完全ビット一致・剛体回転慣性の完全再現 |
| Intent loco は user_root を kinematic；Grab は `GRABBED` + `grab_targets` + SlotGraph / distance clamp | throw impulse・Grab 回転追従・capsule Walk 押し出しの本採用 |
| spawn/despawn は tape membership を直接更新（Rapier 二重登録なし） | Loro CRDT / Formula / StaticSDF asset / Hierarchy Lens |
| `host-server` から `rapier3d` 依存を除去 | workspace 他クレートの Rapier 掃除 |

---

## 設計（要点）

```
Intent / Grab / spawn
        │
        ▼
host barrier: frame_delta→prev → xpbd::step (LBVH body contacts OFF)
  → ground plane (y=0; vertical frame_delta = 0)
  → write frame_delta → publish if moved → commit
        │
        ▼
wire sample from tape Curr
```

- user_root: `inv_mass=0`（kinematic）。mesh / grabbable box: `DYNAMIC`。
- 床は **地面平面**（縦速度は接触で即ゼロ）。動体同士 LBVH 接触は **本 Slice では入れない**（後続）。
- `FrameBridge::tick` は使わず host 独自バリア（ground / delta handoff / idle publish skip）。
- client の Frame→local Epoch（x0090–x0092）は維持。PROTOCOL_PIN は変えない。

---

## Phase

| Phase | 内容 | 状態 |
|:---|:---|:---|
| 0 | Slice + roadmap 目次 | **done** |
| 1 | `PhysicsWorld` / bridge を XPBD 権威化 + 単体 | **done** |
| 2 | `main` Intent/Grab/wire sample 切替 | **done** |
| 3 | Rapier 依存除去・docs・完了条件緑 | **done** |

---

## 完了条件

1. `cargo test -p alchemy-shared-sim` / `cargo test -p host-server` 緑
2. host 経路に Rapier `step` / `tick_mirror_publish` / Rapier pose sample が無い
3. 回帰: coin 床着地オーダー・Grab begin/follow/release・Intent 移動が Frame に出る
4. `cargo test -p client` 既存緑（display 経路非破壊）
5. `host-server` の `Cargo.toml` に `rapier3d` が無い

## やらない（本 Slice）／次

- wire pose ダブルバッファ置換 → **[x0095](x0095-wire-pose-double-buffer.md)** **done**
- Walk カプセル押し出し本番・StaticSDF
- Hierarchy UI / Spatial Lens → **[x0094](x0094-hierarchy-virtual-lens.md)** **done**（Settings 最小；Uber-SDF / 親子同期は後続・空間本線一時停止）
- TPS / Live カメラ

## 関連

- [x008f-host-frame-bridge.md](x008f-host-frame-bridge.md)
- [x008e-zero-base-spatial-lifecycle.md](x008e-zero-base-spatial-lifecycle.md)
- [x0092-eye-display-trs.md](x0092-eye-display-trs.md)

← [目次](../README.md)
