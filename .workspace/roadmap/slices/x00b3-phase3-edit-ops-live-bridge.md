# x00b3 — Phase 3: ライブ確定レーン（Reparent/SetField + FrameBridge・VCS 前提）

> **ID:** `x00b3` · **slug:** `phase3-edit-ops-live-bridge` · **status:** `done` · **area:** `[protocol, host, client, shared-sim, docs]` · **depends_on:** `[x00b2]`  
> **優先度:** [mvcc_xpbd.md](../mvcc_xpbd.md) §4.7 案 B（ライブ tip / 明示 Save）  
> **後続:** 明示 Save（Save Changes）· Spatial VCS · Spawn 即 CAS 見直し · 旧 inventory キー廃止 · Undo UI

目標: Reparent / SetField を **host ライブ確定 + FrameBridge** で適用する。world overlay 即時 CAS はしない（将来の Git 型版管理 = working tree / commit 分離）。`EditCommit.scene_rev` を **host `live_scene_rev`** に再定義し、snapshot に `scene_rev` を載せて gap 収束する。

根拠: [mvcc_xpbd.md](../mvcc_xpbd.md) §4.7 / Q3 · [mvcc_xpbd_review_proposal.md](../mvcc_xpbd_review_proposal.md) 指摘 6 · x00b2

---

## 着手ゲート（満たしていること）

| 条件 | 状態 |
|:---|:---|
| x00b2 `…/edit` Spawn/Despawn | **done** |
| FrameBridge API（`queue_reparent` / `queue_patch`） | shared-sim 既存 |
| Room AEAD / wire_cap / x0024 を壊さない | 本スライスで **契約維持** |

破ってはならないこと: 受信者別 `V_ack`、検証スキップ、Spatial Interest、Undo UI、旧 inventory キー削除、world overlay patch / `SceneEntry.parent` 永続、明示 Save UI。

---

## 保証する / しない（本スライス）

| 保証する | 保証しない |
|:---|:---|
| `EditOp` Reparent / SetField（`expected_parent` / `expected_value_hash`） | world overlay 即時 CAS；`SceneEntry.parent` 永続 |
| **barrier（queue 含む）成功後**に tip+1 と accept `EditCommit`（`scene_rev` = その tip） | accept 前の tip 前進；tip の巻き戻し |
| queue / barrier / live-inbox 失敗時は reject `EditCommit`（`APPLY`、tip 非前進） | 沈黙タイムアウト任せ |
| Spawn/Despawn も Commit の tip は live（overlay CAS は暫定継続；tip は inbox send 成功後） | Spawn と Reparent の永続対称性 |
| Ready → 次 60Hz 境界で FrameBridge 適用（同一 tick の pending は **物理+Commit** バッチ原子；失敗時は atom/topo を戻してから reject） | ティック途中のトポロジ変更；patch 種類が `SetAssetAtom` 以外に増えた場合の汎用補償 |
| `RenderFrame.scene_rev` + client gap → snapshot tip | 欠落 Commit の完全ローカル再適用（host 生存中の tip 合わせ） |
| 同一 child 並行 Reparent は一方のみ；非重複 SetField は両方 | 複数 op を 1 Commit にまとめること；`SetAssetAtom` 以外の live patch 種 |
| `SlotUndoStack::undo_checked`（ライブラリ） | Undo UI / Ctrl+Z |
| `PROTOCOL_PIN` = `…-x00b3` | Spatial Interest / Save As |

---

## VCS 負債（本スライスで直さない・文書必須）

1. Spawn/Despawn の即 overlay CAS（working tree を経ない暫定 autosave）
2. 永続非対称 UX（箱は残る・親子は消える）→ 明示 Save で解消予定
3. 明示 Save / Spatial VCS / Save As・Copy は後続

---

## 実装メモ

* `edit.proto`: `EditReparent` / `EditSetField`；`edit_field_id` FLAGS / COLOR_RGBA / BLAKE3
* `render_frame.proto`: `scene_rev = 9`（snapshot 兼用）
* host `live_scene_rev`；Reparent/SetField は overlay 非接触
* `PhysicsWorld` + `FrameBridge`；Grab `SlotGraph` と創作 `FlatTopology` を混同しない
* client: `scene_rev` 欠番で snapshot；watermark を snapshot tip で合わせる

---

## Phase

| Phase | 内容 | 状態 |
|:---|:---|:---|
| 0 | Slice + roadmap / mvcc_xpbd | **done** |
| 1 | proto + PIN + reject/field_id | **done** |
| 2 | host live_rev + FrameBridge + snapshot | **done** |
| 3 | client gap + 回帰 + DoD | **done** |

---

## 完了条件

1. `PROTOCOL_PIN` が `x00b3` 世代
2. Reparent/SetField roundtrip + 前提失敗で reject
3. accept が FrameBridge 経由で次ティック境界に適用
4. `EditCommit.scene_rev` が live 単調（Spawn でも overlay rev 一致を要求しない）
5. snapshot `scene_rev` + gap テスト
6. overlay patch / Undo UI / 旧キー削除を入れない
7. roadmap 次 = 明示 Save または Spawn CAS 見直し

---

## 起動・検証メモ

```powershell
cargo test -p alchemy-protocol
cargo test -p alchemy-shared-sim
# from client/
cargo test -p host-server --bin host-server
cargo test -p client --bin client
```

推奨ブランチ: `feat/x00b3-phase3-edit-ops-live-bridge`
