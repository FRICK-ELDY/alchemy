# x00f4 — ドラッグ揮発 ↔ Commit 1 回（多軸まとめ）（F1-3）

> **ID:** `x00f4` · **slug:** `drag-volatile-commit` · **status:** `done` · **area:** `[host, client, docs]` · **depends_on:** `[x00f2, x00f3]`  
> **優先度:** 体験本線 **F1-3**（Feel wiring）  
> **体感 1 文:** Inspector で位置をドラッグして離すと、動いた軸がまとめて 1 回だけ確定し、ドラッグ中は共同編集ストアに座標が流れない。

根拠: [feel_edit_experience.md](../feel_edit_experience.md) §4.1 F1-3 · [x00f2](x00f2-inspector-setfield-commit.md) · [scene_store_sim_arena.md](../scene_store_sim_arena.md) §3.4

---

## 保証する / しない（本スライス）

| 保証する | 保証しない |
|:---|:---|
| Desktop Lens POS: ドラッグ中 `…/edit` **0**；離し **1** `EditProposal` | Inspector ドラッグの **Zenoh 揮発ストリーム**（他者可視 Intent/Frame；現状はローカル PendingLocal のみ）→ **後続候補** |
| 同 proposal 内 dirty `POS_{X,Y,Z}` のみ **1..=3** ops（同一 entity・軸重複なし） | **Grab 離しの姿勢 CRDT/SetField Commit**（現行 Epoch/Frame SSoT；Intent 揮発のまま）→ **後続候補** |
| Host: 上記多軸 POS バッチ受理；SoftLease **1**；1 tip / 1 accept Commit | SoftLease ↔ 物理 Grab / `holder_slot` 統合 |
| 2D スクラブ（主軸 `Δu`・環状次軸 `Δv`＋ secondary デッドゾーン）で最大 2 軸 dirty | XR Inspector スクラブ本配線；Save pose 永続 |
| POS 多軸 queue は verify-all-then-push（途中 stale で `pending_pos` 非汚染） | 失敗時 SoftLease 即解放（TTL 満了まで保持；x00e2 / x00f2 継続） |
| Grab hold 中に SetField を出さない（既存 Intent 揮発の非破壊） | 新規クレート；`PROTOCOL_PIN` 変更（proto 既に `repeated EditOp`） |

---

## 後続（未発行・本スライス外）

| 候補 | 内容 | 注意 |
|:---|:---|:---|
| Inspector ネット揮発スクラブ | Lens 正本の「ドラッグ中 Zenoh 揮発 → 離し CRDT 1 回」理想寄せ | CRDT には載せない。SoftLease/Grab SSoT を壊さない |
| Grab 離し姿勢の構造 Commit | §3.4「手放し 1 回」を物理 Grab に寄せる場合 | 現行は Arena/Epoch が pose SSoT。必須化は契約変更 |

## 完了条件

1. 同 entity POS_X+POS_Y を 1 proposal で SoftLease 1・queue 両軸・accept 1。
2. 軸重複 / 異 entity / POS+FLAGS 混在は reject。
3. Desktop scrub: ドラッグ中 publish 0；離し 1 proposal（`ops.len()` == dirty）。
4. 関連テスト Green・roadmap / feel 更新。

## 触ったファイル

- `client/host-server/src/{edit,live_edit,host_loop,sim}.rs` — POS 多 op 受理・Ready バッチ・`queue_live_set_field_ops` / `prepare_pos_queue`
- `client/app/src/spatial_lens/{extract,inspector_edit}.rs` — draft_xyz / 2D scrub / 多軸 Commit
- `client/app/src/{intent_session,surface/desktop/mod}.rs` — sign / Desktop 配線
- [feel_edit_experience.md](../feel_edit_experience.md) / [README.md](../README.md)

← [目次](../README.md)
