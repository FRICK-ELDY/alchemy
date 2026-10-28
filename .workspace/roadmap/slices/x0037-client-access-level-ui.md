# x0037 — client アクセスレベル UI（新規ワールド作成・仮置き）

> **ID:** `x0037` · **slug:** `client-access-level-ui` · **status:** `done` · **area:** `[client, world, docs]` · **depends_on:** `[x0034, x0036, x0018]`

目標: Dash Home から「新規ワールド作成」→ テンプレ（殻）・セッション名・アクセスレベル → ルーム開始で、現行 `ROOM_ID` の Room に visibility を載せたうえで入室する。**見た目は仮置き**（後続で差し替え）。

## 用語

| UI 文言 | サーバ | 意味 |
|:---|:---|:---|
| アクセスレベル | `visibility` | 入場 ACL（描画の表示切替ではない） |

## 設計

| 層 | 内容 |
|:---|:---|
| Home（ログイン後） | `[新規ワールド作成]` / `[入室]` |
| 作成パネル | 左: テンプレ（Grid のみ選択可・他は disabled）／右: セッション名 + アクセスレベル 7 択 + ルーム開始 |
| ルーム開始 | `POST /rooms`（`room_id`=環境 `ROOM_ID` + `visibility`）。409 なら owner なら `PUT …/visibility`。その後 guest Token 入室。非 owner の PUT soft-skip 時は **status で「visibility 未更新」** と明示 |
| ログイン／登録 | セッション確立のみ（自動入室しない）。CI サイレント login は従来どおり入室 |

`room_id` / Zenoh キーの動的切替はしない（`ROOM_ID` 固定）。セッション名は表示用。

**正本 / 起動順:** world DB の `visibility` が正本。host `ROOM_VISIBILITY` は初回 create 既定；owner client PUT が後勝ち。launcher は host 先・**同一アカウント**前提（異 sub が先 create すると host Token 403）。

## やらない

- デザイン最終化・Resonite 見た目の再現
- ポート / Unsafe / Mobile 最適化
- 複数テンプレ・複数シーン
- World 一覧・招待・入室中の途中変更 UI
- `room_id` のランタイム切替（再 Subscribe）

## 成功条件

1. ログイン後 Home から作成パネルを開ける
2. アクセスレベルを選びルーム開始すると world に当該 `visibility` が載り、guest 入室できる（owner 同一アカウント時）
3. architecture / Slice に「仮 UI・アクセスレベル＝visibility」を明記

← [目次](../README.md)
