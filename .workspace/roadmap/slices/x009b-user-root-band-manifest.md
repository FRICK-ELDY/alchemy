# x009b — USER_ROOT 帯を Manifest 配（P2-13）

> **ID:** `x009b` · **slug:** `user-root-band-manifest` · **status:** `done` · **area:** `[protocol, host, world, client, docs]` · **depends_on:** `[x004d]`  
> **優先度:** improvement **P2-13**

目標: user_root の **帯幅（部屋容量）** を protocol 定数 4 に縛らず Manifest の連続 entries から取る。protocol は先頭 ID + DoS 天井だけを持つ。帯を高位へ移し、5 人超でも box ID と衝突しないようにする。

根拠: [improvement-plan-2026-11-05.md](../../0_doc/evaluation/improvement-plan-2026-11-05.md) **P2-13** · [opus-specific-weaknesses-2026-11-05.md](../../0_doc/evaluation/opus/opus-specific-weaknesses-2026-11-05.md) · [opus-specific-proposals-2026-10-30.md](../../0_doc/evaluation/opus/archive/2026-10-30/opus-specific-proposals-2026-10-30.md)

---

## 保証する / しない

| 保証する | 保証しない |
|:---|:---|
| `USER_ROOT_ENTITY_FIRST = 0x1000_0000` + 連続性規則は protocol | UGC で boot 後に slots を動的変更 |
| 部屋容量 = Manifest の連続 user_root entries（`FIRST..FIRST+n-1`、`1..=MAX`） | `USER_ROOT_SLOTS_MAX`（32）超 |
| `USER_ROOT_SLOTS_MAX` は DoS / 固定バッファ天井（部屋サイズ上げに pin 再 bump 不要） | client UBO 16 枠を超える remote 描画 |
| DemoScene は引き続き 4 roots（高位 ID）；5 roots の単体テストあり | 本番チューニングの最適 MAX |
| `PROTOCOL_PIN` → `0.21.0-x009b`（帯位置のピア合意） | 旧 pin ピアとの混在運用 |
| world `next_entity_id` は帯突入時に `FIRST+MAX` へスキップ（`World.Protocol`） | — |

---

## 設計（要点）

```
protocol: FIRST=0x1000_0000, MAX=32 (DoS)
        │
        ▼
Manifest entries: contiguous FIRST..FIRST+n-1  →  n = room capacity
        │
        ▼
host Presence / XPBD: [T; MAX] + slot_count=n
world DemoScene: entity_id = FIRST + i
client draw: remote_reserve = remotes.len() (not MAX)
```

---

## Phase

| Phase | 内容 | 状態 |
|:---|:---|:---|
| 0 | Slice + roadmap / improvement | **done** |
| 1 | protocol 帯移動 + MAX 拡大 + PIN + tests | **done** |
| 2 | DemoScene / world next_id / host / client / vector | **done** |

---

## 完了条件

1. `scene_entry_is_user_root(3..=6)` が false；`FIRST..` が true
2. `user_root_slots_from_entries` が n=5 を受理（MAX 内）
3. DemoScene / Rust mirror / seal vector が新 ID + pin で一致
4. host が 5 Manifest roots で `slot_count=5` になる単体テスト
5. improvement P2-13 を done 表記

## やらない（本 Slice）／次

- bind 全体上限（P2-14）
- DemoScene を本番 5+ 人に増やす（テストで十分）
- host `[T; MAX]` → `Vec` 化

## 関連

- [x004d-user-root-slots-manifest.md](x004d-user-root-slots-manifest.md)
- [improvement-plan-2026-11-05.md](../../0_doc/evaluation/improvement-plan-2026-11-05.md)
- `protocol/src/lib.rs` · `world-server/lib/world/protocol.ex` · `world-server/lib/world/demo_scene.ex`

← [目次](../README.md)
