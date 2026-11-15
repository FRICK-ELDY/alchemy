# x00b6 — Dash Room タブ（薄い B）

> **ID:** `x00b6` · **slug:** `dash-room-tab` · **status:** `done` · **area:** `[client, system_ui, docs]` · **surface:** `[desktop, vr]` · **depends_on:** `[x000f, x007c, x007f]`  
> **優先度:** 明示 Save（後続）の器。Resonite Session タブ相当を **Room** と呼ぶ  
> **後続:** 明示 Save 本体は [x00b7](x00b7-overlay-canonical-save.md)；Users 実データ · Permissions（`editor_subs` HTTP）· visibility 編集 UI · 親子永続（A）

目標: Dash 下部に英語ラベル **`Room`** タブを追加し、入室中だけ開けるようにする。中身は Settings / Users / Permissions の薄いサブメニュー枠。現行 session shell（Leave / Rejoin / host CTA）を Room → Settings へ移し、Save Changes は disabled プレースホルダのみ。

正本: [client.md](../../0_doc/architecture/client.md) · [world-server.md](../../0_doc/architecture/world-server.md) · [mvcc_xpbd.md](../mvcc_xpbd.md) §4.7

## 保証する / しない

| 保証する | 保証しない |
|:---|:---|
| Room タブは `session_shell_active`（Desktop）/ `in_session_peer`（XR）のときだけ開く | overlay→canonical Save、Save As / Copy |
| Session サブ（UI ラベル；enum は `RoomSubTab::Settings`）に Leave / Rejoin / host CTA / room_id | Users 実リスト・kick / mute |
| Users / Permissions は空プレースホルダ | `editor_subs` HTTP CRUD、Moderator |
| Save Changes は disabled + 後続注記 | display_name 永続、visibility 編集 UI |
| 下部タブ順 Home → Inventory → Room → Settings → Exit | 日本語 UI ラベル・CJK フォント |
| Desktop↔XR は既存 `tab` chrome 同期 | `room_sub_tab` の共有（ローカル） |

## 設計確定

| 項目 | 決定 |
|:---|:---|
| UI ラベル | `Room`（概念名は「ルーム」；CJK 非対応のため英語） |
| ゲート | 未入室 → `"Join a room to open Room"`。Inventory の Live ゲートとは逆方向 |
| サブタブ | `RoomSubTab::{ Settings, Users, Permissions }`（UI ラベルは Session / Users / Permissions） |
| session shell | Home/Worlds から外し Room → Session に集約。入室中 Home は短いヒントのみ |
| chrome ゲート | XR は `in_session` / `inventory_live` をタブ適用 **前** に反映（同一更新で Room/Inventory を棄却しない） |
| Widget 再利用 | `WorldsLeave` / `WorldsRejoin` / `WorldsHostRescue` / `WorldsTakeover` |

## Phase

| Phase | 内容 | 状態 |
|:---|:---|:---|
| 0 | Slice 文書 + 目次 | done |
| 1 | `DashTab::Room` / `FacetId` / `WidgetId` / `RoomSubTab` | done |
| 2 | `rebuild_room` + 下部 5 タブ + session shell 移設 | done |
| 3 | 入室ゲート・サブタブ click・Home 分岐 | done |
| 4 | 単体テスト + roadmap / world-server / mvcc 一言 | done |

## やらない（後続）

- 明示 Save（候補 `x00b7`）
- Permissions / Users の実 API
- 個人 Settings（Locomotion 等）の Room への移動

## 起動

```bash
cd client
ALCHEMY_FORCE_DESKTOP=1 cargo run -p client
```

確認:

1. 未入室で Room タブ → 開かない（メッセージ）；Leave 後は Room 選択が Home に戻る
2. Join / Waiting / Live で Room が開き、Session サブに Leave がある
3. Users / Permissions サブが切り替わる（プレースホルダ）
4. Save Changes は押せない
