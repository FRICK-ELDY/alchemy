# x00a2 — SceneBudget（P1-5）

> **ID:** `x00a2` · **slug:** `scene-budget` · **status:** `done` · **area:** `[world, protocol, docs]` · **depends_on:** `[x0082, x009e, x003b]`  
> **優先度:** improvement-plan P1-5（MVCC Phase 1 必須ゲート）

目標: canonical / overlay の次状態に対し、protocol `MAX_MANIFEST_ENTRIES`（4096）/ `MAX_MANIFEST_BYTES`（1 MiB）**以下**で entries 件数と JSON bytes を原子検査する。超過は CAS 前に拒否（413/422）。

根拠: [improvement-plan-2026-11-12.md](../../0_doc/evaluation/improvement-plan-2026-11-12.md) P1-5 · `protocol/src/decode_limits.rs`

---

## 保証する / しない

| 保証する | 保証しない |
|:---|:---|
| `cas_set_scene` / `cas_set_overlay` 直前に次 JSON の entries ≤ 4096 かつ `byte_size` ≤ 1_048_576 | protobuf Manifest とのバイト一致（JSON 側の同数値上限） |
| overlay **および canonical** 変更時に **composed** entries（canon ∥ overlay）≤ 4096 | 既存汚染 DB の自動刈り込み（seal/GET は後続で fail-closed 可） |
| HTTP: `:scene_entries_exceeded` / `:scene_bytes_exceeded` → **413** `scene_budget_exceeded` | host scene RL の時間窓以外の帯域課金 |

---

## Phase

| Phase | 内容 | 状態 |
|:---|:---|:---|
| 0 | Slice + 目次 | **done** |
| 1 | `World.SceneBudget` + CAS 挿入 | **done** |
| 2 | controller エラーマップ + 否定系 | **done** |
| 3 | decode_limits 同期コメント + 保証表 | **done** |

## 完了条件

1. entries=4096 で append → 413；delete 後は成功。
2. 巨大 JSON > 1 MiB → 413。
3. architecture / protocol 注記が一致。
