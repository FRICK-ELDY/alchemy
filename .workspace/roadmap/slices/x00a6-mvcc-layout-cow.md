# x00a6 — MVCC レイアウト予約 + 真の CoW

> **ID:** `x00a6` · **slug:** `mvcc-layout-cow` · **status:** `done` · **area:** `[client, docs]` · **depends_on:** `[x00a5]`  
> **優先度:** [mvcc_xpbd.md](../mvcc_xpbd.md) Phase 2 着手ゲート（レイアウト予約・真の CoW）

目標: `GpuTransferableSoA` / `HotPhysicsPage` に回転変位・慣性・holder 列を予約し、`make_mut` による真の CoW と固定長 `SnapshotSlot`（`MAX_PAGES_PER_SNAPSHOT` 超過は Err）を入れる。剛体回転 XPBD・host/app 本線は触らない。

根拠: [mvcc_xpbd.md](../mvcc_xpbd.md) §4.3 / §5 · [improvement-plan-2026-11-12.md](../../0_doc/evaluation/improvement-plan-2026-11-12.md)

---

## 保証する / しない（本スライス）

| 保証する | 保証しない |
|:---|:---|
| `drot_*` / `inv_inertia_*` 列 + 量子化なしラウンドトリップ | 角速度積分・剛体回転 XPBD |
| `holder_slot` / `holder_since_tick`（`NO_HOLDER`）と `is_grabbed_mask` 整合 | Grab 権威ワイヤ / lease 本線 |
| `make_mut`：履歴共有 or rc>1 なら fork。共有ページへの `page_mut` 拒否 | host/app 配線 |
| `SnapshotSlot` 固定長 + `MAX_PAGES_PER_SNAPSHOT` 超過 `Err` | `delta_codec` / Phase 2 wgpu |
| 既存 `cargo test -p alchemy-shared-sim --lib mvcc` Green | 不等式接触（→ [x00a8](x00a8-inequality-contact.md)） |

## Phase

| Phase | 内容 | 状態 |
|:---|:---|:---|
| 0 | Slice | **done** |
| 1 | page レイアウト + holder API | **done** |
| 2 | arena `make_mut` / SnapshotSlot / テスト | **done** |

## 完了条件

1. `cargo test -p alchemy-shared-sim --lib mvcc` Green（レイアウト往復・`make_mut` 履歴不変・ページ数超過含む）。
2. Epoch / host / app 本線に差分なし。
