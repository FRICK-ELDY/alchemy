# x002a — role → sim capability（D4 残り）

> **ID:** `x002a` · **slug:** `role-sim-capability` · **status:** `done` · **area:** `[protocol, host, world, docs]` · **depends_on:** `[x0029]`

目標: Room Token の `role` を **Intent / world HTTP の capability bitset** に写像し、host が bind 時に保持して Intent 適用でマスクする。権限の正本を「ラベル」から「できること」にする（D4）。

正本: [protocol.md](../../0_doc/architecture/protocol.md) · [world-server.md](../../0_doc/architecture/world-server.md)

## 設計方針

| 層 | 内容 |
|:---|:---|
| protocol | `sim_capability` モジュール。PIN **据え置き**（proto 不変） |
| host | bind で `intent_caps_for_role(role)` を SessionBinding に保存。適用前に `mask_intent` |
| world | HTTP は `role == "host"` 文字列ゲート（Elixir は Rust 表を import 不可）。意味の正本は protocol 表 |

## Capability 表

| bit | 意味 | guest | host |
|:---|:---|:---|:---|
| `INTENT_MOVE` | move_* → Presence overlay | ✓ | ✓ |
| `INTENT_GRAB` | GRAB + aim | ✓ | ✓ |
| `INTENT_LEAVE` | leave reclaim | ✓ | ✓ |
| `HTTP_SCENE` | `GET …/scene` | ✗ | ✓ |
| `HTTP_ATTEST` | `POST …/host_keys` | ✗ | ✓ |
| `HTTP_SEAL` | `POST …/manifest_seal` | ✗ | ✓ |

未知 role → caps `0`（bind 拒否）。

## 保証する

- guest / host の Intent は capability 検査を必ず通る（現状同権でも契約化）
- host のみ HTTP_*（world 既存ゲートと表が一致）
- steady Intent は bind 時 caps を使う（JWT 再読不要）

## 保証しない

- JWT / Presence への `caps` フィールド
- guest と host の Intent 差分（今は同じ INTENT_*）
- **セッション中の role 降格**（caps は bind 時凍結。変更は再 bind）
- world と Rust のコード共有（言語境界。表は文書で同期）
- invite / Friendship / verifier 移譲
- shared-sim

## PIN

据え置き（`alchemy-protocol/0.13.0-x0029`）

## 成功条件

1. `cargo test -p alchemy-protocol -p host-server` 緑  
2. architecture に role→capability 表がある  
3. bind が caps を保持し、マスクが単体テストで確認できる  

← [目次](../README.md)
