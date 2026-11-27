# x00b2 — Phase 3: 確定レーン土台（`…/edit` + EditCommit + Spawn/Despawn 一般化）

> **ID:** `x00b2` · **slug:** `phase3-edit-lane-foundation` · **status:** `done` · **area:** `[protocol, host, relay, client, docs]` · **depends_on:** `[x00b1]`  
> **優先度:** [mvcc_xpbd.md](../archive/2026-11-27/mvcc_xpbd.md) §4.7 確定レーン（inventory 一般化）  
> **後続:** `x00b3` Reparent/SetField + FrameBridge · snapshot `scene_rev` / gap · `undo_checked` · 旧 inventory キー廃止

目標: 既存 inventory spawn/despawn の「提案 → 検証 → 確定」を **`…/edit` + `EditProposal` / `EditCommit`** に一般化する。本スライスは **Spawn / Despawn のみ**。ライブ確定順は host（`scene_rev`）、永続は world overlay CAS（Q3 暫定）。

根拠: [mvcc_xpbd.md](../archive/2026-11-27/mvcc_xpbd.md) §4.7 / §5 Phase 3 · [mvcc_xpbd_review_proposal.md](../archive/2026-11-27/mvcc_xpbd_review_proposal.md) 指摘 6 · x006d / x007b / x0081

---

## 着手ゲート（満たしていること）

| 条件 | 状態 |
|:---|:---|
| dirty 30Hz x00b1 | **done** |
| Room AEAD / wire_cap / x0024 を壊さない | 本スライスで **契約維持** |
| inventory spawn/despawn 制御面が Green | 本スライスで **委譲再利用** |

破ってはならないこと: 受信者別 `V_ack`、独自トランスポート、検証スキップ、Spatial Interest、Undo UI、旧 `…/inventory/*` キーの削除、Reparent/SetField 本線、FrameBridge 全構造変更配線、snapshot への `scene_rev` 追加。

---

## 保証する / しない（本スライス）

| 保証する | 保証しない |
|:---|:---|
| `EditProposal` / `EditCommit` / `EditOp`（Spawn・Despawn）proto | Reparent / SetField / 複数 op バッチ成功 |
| `PROTOCOL_PIN` を `…-x00b2` へ 1 回 bump | 既存 x00af フィールドの再変更 |
| `…/edit` キー + `AeadKind::EditProposal` / `EditCommit` + relay guest put ACL | Spatial Interest |
| accept 時 `EditCommit.scene_rev` = overlay `RoomScene.revision`（CAS 後）を publish | inventory dual-subscribe の EditCommit 発行；`EditCommit` を `RenderFrame` に coalesce |
| Spawn/Despawn は inventory と同等ゲート（AEAD・署名・Dual-auth・rate・`control_seq`） | Invite / 任意 guest ACL |
| 永続は既存 world overlay CAS を再利用（Q3） | world 常時中継の一元権威 |
| client 本線 publish は `…/edit`；host は旧 inventory キーを dual-subscribe | 旧キー削除 |
| client が `EditCommit` を subscribe・decode 可能 | 未確定 Edit の手元予測表示 |
| decode limits + kind 相互開封拒否 | snapshot `scene_rev` / gap 収束 |
| Room AEAD / x0024 / `V_ack` 禁止の維持 | Undo UI / Ctrl+Z / ラジアル |

---

## 実装メモ

* `protocol/proto/edit.proto` — `EditProposal` / `EditOp` / `EditCommit`
* `edit_key` / `EDIT_KEY_LOCAL`；`PROTOCOL_PIN` = `alchemy-protocol/0.22.0-x00b2`
* edit 署名は `wire_crypto` の prost encode（`sig` / `host_sig` クリア；`EditProposal.room_id` を含む）；旧 inventory JSON は `control_plane` 維持
* `EditCommit.scene_rev` = CAS 成功後の packed overlay `RoomScene.revision`（inventory と同空間）。client watermark は **accept のみ** `max(旧, 新)`；未観測の `base_scene_rev` は 0
* inventory dual-subscribe は EditCommit を出さない（互換のみ）
* host `edit.rs`: 単一 Spawn/Despawn → overlay CAS → **Ready enqueue 後**に `EditCommit`；edit/inventory は `SharedControlRate`（8/10s）共有（Spawn blake3/TRS は evaluate/rate 前；`commit_control` 失敗時のみ rate refund；WORLD=CAS 後は seq 消費のまま）
* CAS 後に inbox 閉鎖（shutdown）→ overlay は残し Ready/EditCommit は best-effort
* client `EditCommit` 購読は `host_pub` 必須（fail-closed；Frame/Presence と同型）
* physics 適用は既存 1 tick 1 件（`host_loop`）

---

## Phase

| Phase | 内容 | 状態 |
|:---|:---|:---|
| 0 | Slice + roadmap 目次 | **done** |
| 1 | proto + PIN + AeadKind + ACL + 署名 | **done** |
| 2 | host `…/edit` + scene_rev EditCommit + dual-subscribe | **done** |
| 3 | client 切替 + 回帰 Green + roadmap 次手 | **done** |

---

## 完了条件

1. `PROTOCOL_PIN` が `x00b2` 世代
2. EditProposal Spawn/Despawn の roundtrip + kind 相互拒否
3. host が `…/edit` 経由で overlay CAS → Ready → EditCommit（accept）
4. 旧 inventory キーでも Ready に到達（dual-subscribe）
5. client が EditProposal を put し EditCommit を decode
6. Reparent / Undo UI / Spatial Interest / 旧キー削除を入れない
7. roadmap 次 = `x00b3`（edit ops）

---

## 起動・検証メモ

```powershell
cargo test -p alchemy-protocol
# from client/
cargo test -p host-server --bin host-server
cargo test -p client --bin client
```

推奨ブランチ: `feat/x00b2-phase3-edit-lane-foundation`
