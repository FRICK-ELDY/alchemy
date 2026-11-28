# x00f2 — Inspector スクラブ → SetField Commit（F1-1）

> **ID:** `x00f2` · **slug:** `inspector-setfield-commit` · **status:** `done` · **area:** `[protocol, host, client, docs]` · **depends_on:** `[x00bc, x00e2, x00f1]`  
> **優先度:** 体験本線 **F1-1**（Feel wiring）  
> **体感 1 文:** Inspector の X/Y/Z をドラッグして離すと、その軸が `…/edit` SetField で確定し、オブジェクトがそこに残る。

根拠: [feel_edit_experience.md](../feel_edit_experience.md) §4.1 F1-1 · [x00bc](x00bc-spatial-inspector-lens.md) · SoftLease [x00e2](x00e2-soft-lease-edit-lock.md)

---

## 保証する / しない（本スライス）

| 保証する | 保証しない |
|:---|:---|
| Desktop Lens: POS_X/Y/Z を **rising-edge** press〜release で 1 軸スクラブ → 離し時 1 回 `EditSetField` | readiness（R）編集；quaternion / scale |
| スクラブ UV = **パネル hit UV**；primary 保持中の miss は `last_uv` 継続（press-lock / miss→`[0,0]` 非使用） | SoftLease の UI 見える化（→ F1-2） |
| `edit_field_id::POS_{X,Y,Z}`（f64 LE）＋ SoftLease のみ（**sparse 非書き込み**；Epoch が pose SSoT） | 高頻度揮発ストリーム最適化（後続）・多軸まとめは [x00f4](x00f4-drag-volatile-commit.md) |
| ドラッグ中は CRDT 非送信（Lens draft ＋ PendingLocal）；行死亡時 pending clear | pose の Save 永続（x00b8 非保証のまま） |
| queue 時 Epoch 軸ハッシュ；不一致は head のみ `STALE_PRECONDITION`（abort 尾は APPLY） | XR Inspector スクラブ本配線；新規クレート |
| 離し時 `expected_hash` を Curr 再取得（物理ドリフト許容；Grab 競合は SoftLease 非対象） | 物理 Grab との楽観ロック統合 |
| 既存 FLAGS/COLOR/BLAKE3 SetField・物理 Grab 非破壊 | |

## 完了条件

1. POS_X（および同型 Y/Z）: スクラブ → accept Commit → Frame で pose 反映の経路がある。
2. SoftLease 競合時は既存 `SOFT_LEASE` reject（ゲート流用）。
3. ドラッグ中に `…/edit` を連打しない（release 1 回）。
4. 関連テスト Green・roadmap 更新。

## 触ったファイル

- `protocol/src/{lib,edit_field}.rs` — `POS_X/Y/Z`・`encode_f64` / `hash_f64`（PROTOCOL_PIN 据え置き）
- `client/host-server/src/edit.rs` — POS SoftLease 入口；Epoch ハッシュは queue 時
- `client/host-server/src/live_edit.rs` — POS SoftLease-only（sparse 非書き込み）/ empty rollback
- `client/host-server/src/sim.rs` — queue 照合＋XPBD 後に world 適用＋TELEPORT
- `client/host-server/src/host_loop.rs` — `expected_value_hash` 引き渡し
- `client/app/src/spatial_lens/{extract,inspector_edit,mod,laser}.rs` — scrub draft / Commit publish
- `client/app/src/surface/desktop/mod.rs` — Desktop ドラッグ〜離し配線
- [feel_edit_experience.md](../feel_edit_experience.md) / [README.md](../README.md)

← [目次](../README.md)
