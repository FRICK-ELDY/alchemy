# x00f1 — PendingLocalOps を実操作へ（F0-4）

> **ID:** `x00f1` · **slug:** `pending-local-ops-wire` · **status:** `done` · **area:** `[client, docs]` · **depends_on:** `[x00e6, x00f0]`  
> **優先度:** 体験本線 **F0-4**（Feel wiring）  
> **体感 1 文:** Grab の楽観姿勢が PendingLocalOps にも載り、Published を汚さず表示できる。

根拠: [feel_edit_experience.md](../feel_edit_experience.md) §4.1 F0-4 · [x00e6](x00e6-pending-local-ops.md)

---

## 保証する / しない（本スライス）

| 保証する | 保証しない |
|:---|:---|
| Desktop Grab begin / predict（owned 受理時のみ）/ clear / authority reconcile が `PendingLocalGate` を更新 | Grab `OwnedPrediction` の完全置換（reject grace は残置） |
| Published / EpochSnapshotPair Arc 非汚染（x00e6 契約維持） | Inspector スクラブ → Commit（→ F1-1） |
| Snap / reject で pending エントリ削除 | レーザー／物理ボタン専用経路の新規本配線 |
| | SoftLease 見える化（→ F1-2） |

## 完了条件

1. `SharedState::begin_held_grab` / `predict_held_grab` / `clear_held_grab_prediction` / `reconcile_held_on_authority` が pending_local と同期。
2. 既存 owned_prediction / pending_local テスト Green。新規クレートなし。
3. roadmap 更新。

## 触ったファイル

- `client/app/src/scene.rs` — Grab ↔ PendingLocalGate ミラー
- `client/app/src/pending_local_ops.rs` — x00f1 契約コメント
- [feel_edit_experience.md](../feel_edit_experience.md) / [README.md](../README.md)

← [目次](../README.md)
