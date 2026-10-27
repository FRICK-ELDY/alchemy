# x0030 — shared-sim / EntityKind（P4-2）

> **ID:** `x0030` · **slug:** `shared-sim-entity-kind` · **status:** `done` · **area:** `[sim, host, client, docs]` · **depends_on:** `[x0027]`

目標: host と client で重複していた Manifest 分類・entity 定数を **`alchemy-shared-sim`** に寄せる。Rapier / wgpu / Formula は移さない。

正本: [protocol.md](../../0_doc/architecture/protocol.md) · [client.md](../../0_doc/architecture/client.md)

## 設計方針

| 層 | 内容 |
|:---|:---|
| shared-sim | `EntityKind` / `classify_scene_entry` / `ManifestIndex` / Grab・デモ定数 |
| protocol | `USER_ROOT_*` / `scene_entry_is_*` はワイヤ隣接の薄い契約として残す |
| host | `from_entries` は `classify` で物理構築；球半径は `PHYS_BALL_RADIUS` |
| client | Manifest 適用は `ManifestIndex`；draw fallback は shared-sim 定数 |

## 分類優先度（変更なし・x0027）

1. user_root id 帯 → `UserRoot`
2. `blake3` 非空 → `Mesh`
3. `half_extent > 0` → `Box`（Grab = `flags & GRABBABLE`）
4. それ以外 → `Ignored`

## やらない

- Rapier / 予測補間 / Formula VM の移管
- Elixir `DemoScene` codegen（定数コメントで手動同期）
- threat model（x0031）・launcher（x0032）
- PROTOCOL_PIN 上げ（ワイヤ schema 不変）

## 成功条件

1. host と client の Manifest 分類が shared-sim 一箇所
2. `EntityKind` が型として存在する
3. `cargo test --workspace`（client）緑
4. 本ファイル + roadmap 目次更新

← [目次](../README.md)
