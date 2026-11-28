# x00f3 — SoftLease 見える化（F1-2）

> **ID:** `x00f3` · **slug:** `soft-lease-visibility` · **status:** `done` · **area:** `[protocol, host, client, docs]` · **depends_on:** `[x00e2, x00f2]`  
> **優先度:** 体験本線 **F1-2**（Feel wiring）  
> **体感 1 文:** Inspector を開いたとき、編集ロックを誰が握っているかが chrome で分かり、競合 reject もログ以外で見える。

根拠: [feel_edit_experience.md](../feel_edit_experience.md) §4.1 F1-2 · SoftLease [x00e2](x00e2-soft-lease-edit-lock.md) · Inspector Commit [x00f2](x00f2-inspector-setfield-commit.md)

---

## 保証する / しない（本スライス）

| 保証する | 保証しない |
|:---|:---|
| `EditCommit.lease_holder_actor_id`：SoftLease reject 時は現 holder、Reparent/SetField accept 時は取得 actor（0=なし） | SoftLease 全表の毎 Frame / Epoch 配信 |
| Client: Commit から entity 別ヒントを短命キャッシュ（≈ SoftLease TTL） | Presence 表示名 / CJK / ユーザーアイコン |
| Desktop Spatial Lens chrome: 選択 entity に active hint があれば `LK` + holder DirectNumber；自 hold / 他 hold で色分け | XR Inspector スクラブ本配線；Save 永続 |
| SoftLease 競合 hint を SharedState 経由で Lens が読める | 物理 Grab / `holder_slot` / `Entity.holder` との統合 |
| accept Commit の holder は **acquiring actor を Ready に固定**（表再参照しない；同 tick clear でも 0 にしない） | host Hz ≠ 60 時の wall-clock hint TTL 厳密一致 |
| `PROTOCOL_PIN` = `alchemy-protocol/0.25.0-x00f3` | chrome は holder **下位 32bit** のみ（Zero-String）；Presence 名 |
| Despawn accept で entity hint を消す | ドラッグ揮発↔Commit 最適化は [x00f4](x00f4-drag-volatile-commit.md)；ネット揮発／Grab 離し Commit は後続 |

## 完了条件

1. SoftLease reject の `EditCommit` に現 holder が載る（0 でない）。
2. SoftLease ゲート成功の accept Commit にも holder が載る。
3. Lens chrome で選択 entity の SoftLease（mine / other）が色と `LK`+数値で分かる。
4. 物理 Grab 経路を変更しない。
5. 関連テスト Green・roadmap 更新。

## 触ったファイル

- `protocol/proto/edit.proto` / `protocol/src/{lib,demo_scene_vector,wire_crypto}.rs` — `lease_holder_actor_id`・PIN・vector
- `client/host-server/src/{edit,live_edit,host_loop}.rs` — reject/accept に holder
- `client/app/src/{soft_lease_hint,bus/edit,scene,main,spatial_lens/*,surface/desktop/mod}.rs` — hint＋chrome
- `world-server` config / DemoSceneVector / canonical_manifest テスト
- [feel_edit_experience.md](../feel_edit_experience.md) / [README.md](../README.md)

← [目次](../README.md)
