# x00a8 — 不等式接触（引き寄せ禁止）

> **ID:** `x00a8` · **slug:** `inequality-contact` · **status:** `done` · **area:** `[client, docs]` · **depends_on:** `[x00a4]`  
> **優先度:** [mvcc_xpbd.md](../mvcc_xpbd.md) P1-2 / 🚫6（等式距離流用禁止）

目標: LBVH 接触を `FLAG_INEQUALITY` にし、離れている物体を引き寄せない。`contact_never_attracts` 回帰。host `bounds: Some` は Phase 3。

## 保証する / しない

| 保証する | 保証しない |
|:---|:---|
| `dist >= rest` で接触ソルブ no-op | 箱 vs 箱の正確な OBB；host 本線オン |
| 既存 2 箱重なり解消回帰 | 外接球近似の撤廃 |

## 完了条件

1. `cargo test -p alchemy-shared-sim --lib xpbd` / `mvcc` Green。
