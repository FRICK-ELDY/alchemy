# x0029 — RenderFrame 回転 + スケール（E1）

> **ID:** `x0029` · **slug:** `renderframe-trs` · **status:** `done` · **area:** `[protocol, host, client, docs]` · **depends_on:** `[x0028]`

目標: `RenderFrame.Entity` に **単位クォータニオン + 非一様スケール** を載せ、host が Rapier 姿勢を出し、client が描画 TRS に適用する。定義サイズ（Manifest `half_extent`）は残し、Frame scale は実行側の倍率とする（D2 / E1）。

正本: [protocol.md](../../0_doc/architecture/protocol.md) · [peer-delivery.md](../../../protocol/peer-delivery.md)

## 設計方針

| 層 | 内容 |
|:---|:---|
| protocol | `Entity` に `qx..qw`（xyzw）と `sx,sy,sz`。PIN → `0.13.0-x0029` |
| host | Rapier `translation` + `rotation` を Pose に載せる。scale は当面 `(1,1,1)`（剛体に scale 無し） |
| client | `EntityPose` が TRS を保持。box/mesh/remote 描画は `from_scale_rotation_translation`。box 見た目 = `half_extent*2 * frame_scale` |
| ゼロ値 | quat 全ゼロは不正 → 発行側は identity `(0,0,0,1)`。scale 全ゼロは unset → 消費側 `(1,1,1)` |

## 保証する

- 有限な quat / scale が `entity_is_finite` / Frame 署名経路に含まれる
- 回転のみの変化でもデルタ `upserts` に載る（`ROT_EPSILON`）
- Manifest / Presence の `protocol_pin` 不一致で旧 peer を拒否（既存 x0023）

## 保証しない

- Grab ヒットの OBB → **x002b で解消**（当時は AABB + 位置×scale）
- Frame scale アニメ / コライダーの非一様リサイズ
- Manifest `SceneEntry` への初期 quat（定義側の回転は後続）
- shared-sim / EntityKind
- Grab aim への **回転**追従（host は translation のみ）

## PIN

`alchemy-protocol/0.13.0-x0029`

## 成功条件

1. `cargo test -p alchemy-protocol -p host-server -p client` 緑  
2. architecture / roadmap が Entity TRS を正とする  
3. host/client 起動バナーが新 PIN を出す  

← [目次](../README.md)
