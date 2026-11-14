# x00a7 — EntityDirectory + セル越え

> **ID:** `x00a7` · **slug:** `entity-directory` · **status:** `done` · **area:** `[client, docs]` · **depends_on:** `[x00a6]`  
> **優先度:** [mvcc_xpbd.md](../mvcc_xpbd.md) §4.3 EntityDirectory（Phase 1 P0）

目標: wire 正 ID `entity_id: u32` と `PageSlot` / `CellId` を導入し、セル越えを同一呼び出し内で原子的に行う。Q8 暫定: `CELL_SIZE_M=64` / `CELL_HYSTERESIS_M=2`。

根拠: [mvcc_xpbd.md](../mvcc_xpbd.md) · [spatial_interest.md](../spatial_interest.md)（キー系のみ共有）

---

## 保証する / しない

| 保証する | 保証しない |
|:---|:---|
| エンティティは高々 1 `PageSlot` | 配信 interest / AOI 購読 |
| `cross_cell` で src 非活性 → dest 活性が原子的 | host 本線・マルチページ負荷分散 |
| ヒステリシスで境界フラップしない | 巨大建造物の複数セル占有 |

## 完了条件

1. `cargo test -p alchemy-shared-sim --lib mvcc::directory` Green。
2. host/app 本線差分なし。
