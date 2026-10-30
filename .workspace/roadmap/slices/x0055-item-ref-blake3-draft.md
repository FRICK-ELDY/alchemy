# x0055 — ItemRef（blake3）草案

> **ID:** `x0055` · **slug:** `item-ref-blake3-draft` · **status:** `planned` · **area:** `[protocol, docs]` · **depends_on:** `[x0054]`  
> **ladder:** Wave 0 · [inventory-ladder.md](../inventory-ladder.md)

目標: host／client／assets が共有する **スポーン参照の型**を `blake3` のみで草案化する。ワイヤ配送・spawn 実行はまだしない。

## 保証する / しない

| 保証する | 保証しない |
|:---|:---|
| `ItemRef` が「CAS blake3 を指す」契約として文書＋型（Rust および／または proto）に存在する | `repo_id@rev`（→ [x0063](x0063-item-ref-repo-rev.md)） |
| schema_version / 将来拡張点のコメント | Manifest / Frame / Intent への載荷 |
| PROTOCOL_PIN 方針が Slice 内で明示（載荷するときだけ bump） | 実 spawn・権限チェック実行 |

## 設計メモ

- host は永続 `node_id` を持たない。知るのは実行時 `entity_id` とスポーン時 `ItemRef`
- C+3 で `repo@rev` を足すため、blake3 専用フィールドを後から壊さない形にする

| Phase | 内容 | 状態 |
|:---|:---|:---|
| 0 | 本 Slice | planned |
| 1 | 型＋単体テスト（encode/decode または純 Rust） | planned |
| 2 | protocol.md / ladder 注記 | planned |

## 完了条件

1. ItemRef（blake3）がコードまたは proto にありテスト緑
2. 既存 Manifest / Frame 互換を壊さない（未載荷なら PIN 非 bump 可）

## やらない

- take-out / put-back
- DriveNode
- Vcs

← [目次](../README.md) · [ladder](../inventory-ladder.md)