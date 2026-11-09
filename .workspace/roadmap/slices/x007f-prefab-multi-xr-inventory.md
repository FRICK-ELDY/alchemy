# x007f — Prefab 多子展開 + XR Inventory 薄皮

> **ID:** `x007f` · **slug:** `prefab-multi-xr-inventory` · **status:** `done` · **area:** `[assets, client, system_ui, docs]` · **depends_on:** `[x005e, x006d, x006e, x007b, x007c]`  
> **優先度:** backlog「空間で物を扱う」体感（Prefab 多子 + XR Inventory 薄皮）

目標: Prefab.Payload の **多子 Tree** を `take_out` で子 mesh 列に展開し、client が既存単一 blake3 spawn を **逐次**打って部屋に並べる。Desktop は多選択 put-back。Inventory は **Live 中のみ**解禁。XR は Desktop へ Inventory 操作を委譲する（Zenoh / assets HTTP 正本は Desktop）。

根拠: [inventory-ladder.md](../inventory-ladder.md) Wave 3 残ギャップ · [x005f](x005f-inventory-take-out.md) Prefab 展開目標 · [x006d](x006d-host-take-out-control.md) / [x006e](x006e-inventory-put-back-ui.md) 単一子縮小 · [backlog-priority-2026-11-04.md](../backlog-priority-2026-11-04.md)

---

## 保証する / しない

| 保証する | 保証しない |
|:---|:---|
| assets: prefab `take_out` が Payload を読み **子 mesh**（`blake3` + `local_id` + 任意 `trs`）を返す；各子と payload に Grant 必須；欠落は全体 `:no_grant` | payload 自身の blake3 を blobs に載せる；folder / `vcs_repo` take-out |
| blob `take_out` は従来どおり単一 `%{blake3}` | host 一括多子 spawn / SceneEntry 契約変更 |
| Desktop: prefab 選択 → 子ごとに既存 `inventory/spawn`；host `SpawnRate(8/10s)` ペースは **UI 外ワーカー**；途中失敗で停止し `spawn put N/M` status（world ロールバックなし）。Leave/Logout は epoch discard で中断し Mirror を `take_out cancelled` に落とす | 原子的 all-or-nothing spawn；host accept のクライアント観測；専用 inventory job（auth_job 共用のまま） |
| Desktop: scene mesh **多選択** put-back → 多子 Prefab.Payload | put-back + despawn 一体 TX；名前 UI 完成 |
| Inventory は **Live のみ**解禁；Waiting / KeyStale / Entering は遮断のまま | Waiting 中の take-out |
| XR Inventory: 選択・entity_id を `InventoryPeerRequest` に載せ Desktop drain；Drive 一覧は `InventoryMirror` で Desktop→XR | XR 単独の assets/Zenoh；Intent 経路；`repo@rev`；host rate 緩和 |

---

## 設計（要点）

```
assets Drive prefab ─take_out─► children[{blake3, local_id, trs?}]
client Desktop ─N× Zenoh inventory/spawn─► host (単一 blake3 据え置き)
XR Dash ─pending─► Desktop drain ─► 同上
scene meshes (multi-select) ─put_back─► Prefab.Payload children[]
```

- 定義 = assets；実行 = client が host の既存 1:1 spawn を繰り返す
- 部分成功: 途中失敗で停止・既 spawn は残す（host 非保証と一致）

---

## Phase

| Phase | 内容 | 状態 |
|:---|:---|:---|
| 0 | Slice + 目次 / backlog | **done** |
| 1 | assets: prefab take_out 展開 + Grant + テスト | **done** |
| 2 | Desktop: 逐次 spawn・多選択 put-back・Live Inventory | **done** |
| 3 | XR→Desktop Inventory 委譲 | **done** |
| 4 | architecture 保証表 + 回帰 | **done** |

---

## 完了条件

1. owner が prefab DriveNode を take_out → 部屋に複数 mesh（Grant 無し子は全体失敗）
2. Desktop 多選択 put-back → 多子 Prefab が Drive に増える
3. Live 中 Inventory 可；Waiting/KeyStale では遮断
4. XR で行選択 → Take out / Put back / Despawn が Desktop 経由で効く（選択ペイロード＋Mirror）
5. blob 単体 take-out・host 単一 blake3 契約の回帰が通る
6. 多子 take_out の status は `spawn put`（ペース明示）；host 8/10s をクライアントが守る

## やらない（本 Slice）

- host 一括多子 spawn / SceneEntry 親フィールド
- `repo@rev` take-out；フォルダ UX
- assets 別リポ化 / 公開体裁専用 PR
- put-back+despawn 一体 TX

← [目次](../README.md)
