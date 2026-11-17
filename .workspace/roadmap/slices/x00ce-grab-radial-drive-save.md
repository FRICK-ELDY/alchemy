# x00ce — グラブ中 Prefab をラジアル Save → Drive

> **ID:** `x00ce` · **slug:** `grab-radial-drive-save` · **status:** `todo` · **area:** `[client, assets, docs]` · **depends_on:** `[x006e, x00b4, x00cd]`  
> **優先度:** 体験確定（掴む → ラジアル → 保存 → Drive）· [assets-vcs-draft.md](../../0_doc/architecture/assets-vcs-draft.md) §18.7 の UI 先行切り出し  
> **後続:** Drive 一覧選択 → Git 詳細パネル（vcs-store / HTTP push）；shared-sim 部分木⇔Tree の本格変換；clone/fetch

目標: **保持中（Grab）の Prefab／mesh 群をラジアルの「保存」で Drive に `put_back` する**導線を一本化する。Room canonical Save（x00b7）や Spatial VCS commit とは別経路。

根拠: 体験合意（2026-11-17）· [x006e](x006e-inventory-put-back-ui.md) put_back · [x00b4](x00b4-prefab-child-grant-required.md) Grant · [client.md](../../0_doc/architecture/client.md) Grab / Inventory

---

## 体験（確定）

1. 保存したい Prefab をグラブする  
2. ラジアルメニューを開く  
3. 「保存」を押す  
4. Drive に Prefab ノードとして保存される（既存 `POST /api/v1/drive/put_back`）

後段（本 Slice 外）: Drive 一覧でそのアイテムを選ぶ → Git 操作の詳細パネル。

---

## 保証する / しない

| 保証する | 保証しない |
|:---|:---|
| Grab 保持中にラジアル「保存」が出せる（Desktop + XR） | Room「Save Changes」（x00b7 overlay→canonical） |
| 保持 entity（またはその Prefab 部分木）から put_back 用 children（blake3 + TRS）を組み立てる | Spatial VCS commit / push / promote（Git 詳細パネル後続） |
| 成功時 Drive に prefab ノードが増える（既存 `save_prefab` / Grant 必須） | put_back と同時 despawn の原子 TX |
| 非保持・Grant 不足・対応づけ不能は fail-closed（保存ボタン無効 or 明示エラー） | 他人ワールド mesh の無断 Drive 取り込み |
| Inventory の既存 Put back は残す（多選択経路） | フル clone/fetch；fork / PR |

---

## 設計メモ

### 現状ギャップ

| 部品 | 状態 |
|:---|:---|
| Desktop / XR `held_grab` | あり（Intent / 物理） |
| Inventory Put back → HTTP put_back | あり（Dash 多選択） |
| ラジアルメニュー | **なし**（本 Slice で新規） |
| Grab → 選択／Save 連動 | **なし** |

### 方針

- **永続先は Drive**（`Assets.Drive.save_prefab` via `put_back`）。VCS は触らない  
- ラジアルは **Grab 保持中専用の薄いアクション輪**（Dash 全体を開かない。現行「Dash 開で Grab 解放」と衝突させない）  
- 保持 entity → blake3 / TRS の解決は既存 Inventory Put back と同じ HTTP 契約に寄せる（`inventory.rs` の `put_back_prefab_children` 再利用）  
- 名前は自動（現行 `putback-…` 系）でよい；リネーム UI は後続  
- XR は既存どおり Desktop へ peer 委譲してよい（HTTP は Desktop 側）

### Room Save / VCS との境界

```text
Grab + ラジアル Save  →  Drive Prefab（本 Slice）
Room Save Changes    →  canonical scene（x00b7・触らない）
Drive 詳細 Git パネル →  vcs-store / push（後続 Slice）
```

| Phase | 内容 | 状態 |
|:---|:---|:---|
| 0 | 本 Slice 文書 | done |
| 1 | ラジアル UI（保持中のみ・Save アクション） | todo |
| 2 | held_grab → put_back children 組み立て + HTTP | todo |
| 3 | Desktop / XR 配線 + 失敗表示 | todo |
| 4 | テスト + docs 鮮度 | todo |

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
