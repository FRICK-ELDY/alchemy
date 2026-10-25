# x0018 — client アカウント殻（login / logout / 最小 register）

> **ID:** `x0018` · **slug:** `client-auth-session` · **status:** `planned` · **area:** `[client, system_ui, docs]` · **surface:** `[desktop]` · **depends_on:** `[x0017]`

目標: auth-server の **既存 API を叩く client セッション殻**を載せる。起動時（または Dash）で **login / 最小 register**、セッション中に **logout**。未ログインでは Room Token 取得・入室をしない。`AUTH_USER` サイレント login は **CI / smoke 用フォールバック**として残してよい。

正本: [auth-server.md](../../0_doc/architecture/auth-server.md) · [client.md](../../0_doc/architecture/client.md)

## 設計方針

| 層 | 役割 |
|:---|:---|
| auth-server | **変更なし**（`/register` `/login` `/refresh` `/logout` `/me` 既存） |
| client | セッション状態（access ± refresh）・login/register/logout 呼び出し・未認証ゲート。Room Token はログイン後のみ |
| system_ui | Desktop Dash（または起動ゲート）に最小フォーム / ボタン。OpenXR 専用 UI は本 Slice 外 |
| host | 変更なし（Intent TTL 解放は x0016 のまま。logout 後は Intent 停止 → TTL でスロット解放） |

| Phase | 内容 | 状態 |
|:---|:---|:---|
| 0 | Slice 文書 + 目次 | done |
| 1 | client: auth HTTP ラッパ + セッション型（login / register / logout / 任意 refresh） | planned |
| 2 | Desktop UI: ログイン／最小登録／ログアウト | planned |
| 3 | 未ログイン入室ゲート + smoke + README / 正本 | planned |

## セッション契約

| 操作 | API | 備考 |
|:---|:---|:---|
| login | `POST /api/v1/auth/login` | username or email + password。任意 remember_me → refresh 保持 |
| register（最小） | `POST /api/v1/auth/register` | username / email / password / birthday / `tos_agreed=true`。成功時セッション発行をそのまま使う |
| logout | `POST /api/v1/auth/logout`（Bearer） | ローカルトークン破棄必須。API 失敗でもローカルは捨てる |
| refresh | `POST /api/v1/auth/refresh` | remember_me 時のみ。期限切れ access の再取得（最小実装で可） |

- 入室: access JWT → world `guest` Room Token → 既存 Presence / Intent 経路（x0015–x0017）
- logout 後: Intent 送信停止・Room Token 破棄・`local_user_root` unset。host 側の即時 Leave はしない（TTL）
- CI: `AUTH_USER` + `AUTH_PASSWORD`（または `ALCHEMY_AUTH_TOKEN`）があれば UI をスキップして従来どおり入室可

## 保証する / しない

| 保証する | 保証しない |
|:---|:---|
| Desktop で login → 入室、logout → 再入室不可（再 login まで） | OpenXR 専用ログイン UI・仮想キーボード完成形 |
| 最小 register（必須フィールドのみ）→ 即セッション | メール確認 UI・再送・forgot/reset password UI |
| logout でローカル JWT/Room Token 破棄 | host への明示 Leave / 即時 Presence 削除 |
| smoke: 登録または login → logout → 再 login | OAuth / Friendship / アカウント削除 UI / launcher 永続クレデンシャル金庫 |

## やらない（後続）

- メール確認・パスワードリセットの client UI
- VR ログインシェル
- 明示 Leave / Zenoh liveliness
- Steal / 投げ / アバター GLB
- launcher 本実装（トークンを OS 鍵束へ）

## 起動（同一マシン）

```bash
# auth / world / assets + zenohd は従来どおり
cd client
cargo run -p host-server

# UI 経由（x0018）: 起動後に login または register
ALCHEMY_FORCE_DESKTOP=1 cargo run -p client

# CI / 従来 smoke（フォールバック）
ALCHEMY_FORCE_DESKTOP=1 AUTH_USER=alice cargo run -p client
```

## 実装メモ

- auth-server の register 必須: `username` `email` `password` `birthday` `tos_agreed`
- UI は既存 Dash Facet か起動前ゲートのどちらか一方に寄せ、固有概念を render 層へ漏らさない
- トークン保持はプロセス内メモリで可。ディスク永続は任意・保証外
- smoke: login→logout の HTTP 単位、または Desktop 手動チェックリストを Slice に残す

← [目次](../README.md)
