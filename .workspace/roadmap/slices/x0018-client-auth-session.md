# x0018 — client アカウント殻（login / logout / 最小 register）

> **ID:** `x0018` · **slug:** `client-auth-session` · **status:** `done` · **area:** `[client, system_ui, docs]` · **surface:** `[desktop]` · **depends_on:** `[x0017]`

目標: auth-server の **既存 API を叩く client セッション殻**を載せる。Dash 下タブに **Home** を追加し、**login / 最小 register / logout**。未ログインでは Room Token 取得・入室をしない。`AUTH_USER` / `ALCHEMY_AUTH_TOKEN` サイレント login は **CI / smoke 用フォールバック**。（完了）

正本: [auth-server.md](../../0_doc/architecture/auth-server.md) · [client.md](../../0_doc/architecture/client.md)

## 設計方針

| 層 | 役割 |
|:---|:---|
| auth-server | **変更なし**（`/register` `/login` `/refresh` `/logout` `/me` 既存） |
| client | `auth` モジュール + `AuthGate`。Room Token はログイン後のみ。Intent は `room_token` ゲート |
| system_ui | 下タブ **Home · Settings · Exit**。Home にフォーム（Desktop キーボード入力） |
| host | 変更なし（logout 後 Intent 停止 → TTL でスロット解放） |

| Phase | 内容 | 状態 |
|:---|:---|:---|
| 0 | Slice 文書 + 目次 | done |
| 1 | client: auth HTTP ラッパ + セッション型 | done |
| 2 | Desktop UI: Home タブ login／register／logout | done |
| 3 | 未ログイン入室ゲート + smoke + README / 正本 | done |

## セッション契約

| 操作 | API | 備考 |
|:---|:---|:---|
| login | `POST /api/v1/auth/login` | username or email + password。任意 remember_me |
| register（最小） | `POST /api/v1/auth/register` | username / email / password / birthday / `tos_agreed=true` |
| logout | `POST /api/v1/auth/logout`（Bearer） | ローカルトークン破棄必須。API 失敗でもローカルは捨てる |
| refresh | `POST /api/v1/auth/refresh` | ヘルパあり・本 Slice では自動再取得なし |

- 入室: access JWT → world `guest` Room Token → Presence / Intent
- logout 後: Intent 送信停止・Room Token 破棄・`local_user_root` unset（host 即時 Leave なし）
- CI: `AUTH_USER`（+ 任意 `AUTH_PASSWORD`）または `ALCHEMY_AUTH_TOKEN` で UI スキップ

## 保証する / しない

| 保証する | 保証しない |
|:---|:---|
| Desktop Home タブで login → 入室、logout → 再入室不可 | OpenXR 専用ログイン UI・仮想キーボード完成形 |
| 最小 register → 即セッション | メール確認 UI・forgot/reset password UI |
| logout でローカル JWT/Room Token 破棄 | host への明示 Leave / 即時 Presence 削除 |
| smoke: HTTP login→logout + Home tab units | OAuth / Friendship / launcher クレデンシャル金庫 |

## やらない（後続）

- メール確認・パスワードリセットの client UI
- VR ログインシェル（Home タブは見えるが文字入力は Desktop）
- 明示 Leave / Zenoh liveliness
- Steal / 投げ / アバター GLB
- launcher 本実装

## 起動

```bash
# UI 経由: AUTH_USER を付けない → Home タブが開く
ALCHEMY_FORCE_DESKTOP=1 cargo run -p client

# CI / 従来 smoke
ALCHEMY_FORCE_DESKTOP=1 AUTH_USER=alice cargo run -p client
```

## 実装メモ

- 下タブ: Home（`home.png`）· Settings · Exit
- `SharedState.auth: AuthGate` — Intent publisher は `room_token` 無しなら無送信
- smoke: `.scripts/smoke-client-auth.sh`

← [目次](../README.md)
