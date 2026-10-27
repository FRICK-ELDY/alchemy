# x002c — Manifest 初期 quat + scale

> **ID:** `x002c` · **slug:** `manifest-initial-trs` · **status:** `done` · **area:** `[protocol, world, host, client, docs]` · **depends_on:** `[x0029, x0027]`

目標: `SceneEntry` に **初期向き（xyzw）と初期 Frame scale** を載せ、World DemoScene → host Rapier spawn / Frame seed → client Vacant EntityPose まで通す。コライダー定義サイズ（`half_extent`）は変えず、scale は描画 TRS のみ（x0029 契約の延長）。

正本: [protocol.md](../../0_doc/architecture/protocol.md) · [world-server.md](../../0_doc/architecture/world-server.md)

## 設計方針

| 層 | 内容 |
|:---|:---|
| protocol | `SceneEntry` に `qx..qw` / `sx..sz`。PIN → `0.14.0-x002c`。ゼロ quat→identity、ゼロ scale→`(1,1,1)` |
| world | `DemoScene` が identity TRS を明示。中央 Grab box は yaw 45° + `sx=1.5` でデモ |
| host | Rapier builder に Manifest rotation。`scales` マップを Frame Pose に載せる（collider は `half_extent` のまま） |
| client | Manifest 適用時 Vacant のみ quat/scale を seed。Occupied（Frame 所有）は tint のみ更新 |

## 保証する

- Manifest の初期 quat / scale が host spawn と Frame upsert / client 初回 pose に反映される
- proto3 ゼロ値は消費側で identity に正規化（`scene_entry_quat_xyzw` / `scene_entry_scale`）
- 旧 PIN peer は Manifest `protocol_pin` 不一致で拒否（既存 x0023）

## 保証しない

- Rapier コライダーの非一様リサイズ（scale≠1 でも `half_extent` 固定）
- **client OBB / 描画は Frame scale、host 物理・Grab 距離は `half_extent` のみ**（デモ中央 box `sx=1.5` で見た目ヒットと物理がズレ得る）
- Manifest revision 上げでの Occupied EntityPose（quat/scale）再シード — Frame 所有を優先、tint のみ更新
- Frame 実行中の scale アニメ / Grab 中の scale 追従変更
- shared-sim / EntityKind / UGC 編集 UI
- Elixir ↔ Rust DemoScene の自動共有 fixture（手動同期のまま・x0027 / x002c debt）

## PIN

`alchemy-protocol/0.14.0-x002c`

## 成功条件

1. `cargo test -p alchemy-protocol` および host-server / client 関連テスト緑  
2. world-server `DemoScene` テストが中央 box TRS を検証  
3. architecture / roadmap が SceneEntry 初期 TRS を正とする  
4. host/client 起動バナーが新 PIN を出す  

← [目次](../README.md)
