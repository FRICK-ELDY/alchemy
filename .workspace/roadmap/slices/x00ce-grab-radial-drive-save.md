# x00ce — グラブ中 Prefab をラジアル Save → Drive

> **ID:** `x00ce` · **slug:** `grab-radial-drive-save` · **status:** `done` · **area:** `[client, assets, docs]` · **depends_on:** `[x006e, x00b4, x00cd]`  
> **優先度:** 体験確定（掴む → ラジアル → 保存 → Drive）· [assets-vcs-draft.md](../../0_doc/architecture/assets-vcs-draft.md) §18.7 の UI 先行切り出し  
> **後続:** Drive 一覧選択 → Git 詳細パネル（vcs-store / HTTP push）；shared-sim 部分木⇔Tree の本格変換；clone/fetch

目標: **保持中（Grab）の Prefab／mesh 群をラジアルの「保存」で Drive に `put_back` する**導線を一本化する。Room canonical Save（x00b7）や Spatial VCS commit とは別経路。

根拠: 体験合意（2026-11-17）· [x006e](x006e-inventory-put-back-ui.md) put_back · [x00b4](x00b4-prefab-child-grant-required.md) Grant · [client.md](../../0_doc/architecture/client.md) Grab / Inventory · Resonite Context Menu 操作

---

## 体験（確定）

1. Context ラジアルを開く（Desktop: 中ボタン / T · XR: B / Y — Dash とは別ボタン）  
2. （任意）保存したい Prefab をグラブした状態で開く／開いたままグラブ  
3. 中央を Primary で押し、Save セクタへフリップ通過  
4. Drive に Prefab ノードとして保存される（既存 `POST /api/v1/drive/put_back`）

後段（本 Slice 外）: Drive 一覧でそのアイテムを選ぶ → Git 操作の詳細パネル。

---

## 保証する / しない

| 保証する | 保証しない |
|:---|:---|
| Context ラジアル（Resonite 風・フリップ通過・手切替） | Room「Save Changes」（x00b7 overlay→canonical） |
| Grab 保持中に「保存」が有効（Desktop + XR peer） | Spatial VCS commit / push / promote（Git 詳細パネル後続） |
| 保持 entity から put_back 用 children（blake3 + TRS）を組み立てる | put_back と同時 despawn の原子 TX |
| 成功時 Drive に prefab ノードが増える（既存 `save_prefab` / Grant 必須） | 他人ワールド mesh の無断 Drive 取り込み |
| 非保持・Grant 不足・対応づけ不能は fail-closed（保存無効 or 明示エラー） | フル clone/fetch；fork / PR |
| Inventory の既存 Put back は残す（多選択経路） | サブメニュー階層・汎用ツール輪 |

---

## 設計メモ

### 方針

- **永続先は Drive**（`Assets.Drive.save_prefab` via `put_back`）。VCS は触らない  
- ラジアルは Dash と独立（`suppress` / Grab 解放と衝突させない）  
- 保持 entity → blake3 / TRS は `inventory::put_back_children_for_entities`（Inventory Put back と共有）  
- 名前は自動（`putback-…`）；リネーム UI は後続  
- XR は Desktop へ `InventoryPeerRequest::PutBack` 委譲  
- OpenXR: **A = Dash** · **B/Y = Context** · trigger = Primary（Resonite 寄せリマップ）

### Room Save / VCS との境界

```text
Context ラジアル Save  →  Drive Prefab（本 Slice）
Room Save Changes    →  canonical scene（x00b7・触らない）
Drive 詳細 Git パネル →  vcs-store / push（後続 Slice）
```

| Phase | 内容 | 状態 |
|:---|:---|:---|
| 0 | 本 Slice 文書 | done |
| 1 | Context ラジアル UI（フリップ・手切替・Save） | done |
| 2 | held_grab → put_back children 組み立て + HTTP | done |
| 3 | Desktop / XR 配線 + 失敗表示 | done |
| 4 | テスト + docs 鮮度 | done |

---

## 完了条件

1. Grab 中にラジアルを開き「保存」→ Drive に prefab が増える（Grant あり）  
2. 未保持 / Grant 不足では保存できない（ボタン無効 or エラー）  
3. Room Save Changes の挙動・経路を変えない  
4. Slice / README / draft §18.7 言及 / client.md 鮮度一致  

## やらない

- Drive 一覧の Git 詳細パネル  
- `vcs-store` commit / push / promote の自動実行  
- shared-sim ⇔ Tree の本格変換（VCS 用）  
- ラジアルの汎用ツール輪（Primary/Context 全体）  
- put_back + 部屋からの削除の一括 TX  

← [目次](../README.md) · [x00cd](x00cd-vcs-http-store.md) · [x006e](x006e-inventory-put-back-ui.md) · [assets-vcs-draft](../../0_doc/architecture/assets-vcs-draft.md)
