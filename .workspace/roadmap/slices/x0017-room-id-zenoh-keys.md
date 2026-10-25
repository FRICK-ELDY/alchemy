# x0017 — ROOM_ID → Zenoh キー本配線

> **ID:** `x0017` · **slug:** `room-id-zenoh-keys` · **status:** `planned` · **area:** `[host, client, protocol, docs]` · **surface:** `[desktop, vr]` · **depends_on:** `[x0016]`

目標: host / client が既に読む **`ROOM_ID`（既定 `local`）を、実際の Zenoh pub/sub キーに反映**する。`*_KEY_LOCAL` 定数への直結をやめ、protocol の `*_key(room_id)` ヘルパ経由に統一。異なる `ROOM_ID` のセッションはキー空間で隔離されることまで smoke する。

正本: [protocol.md](../../0_doc/architecture/protocol.md) · [client.md](../../0_doc/architecture/client.md) · [zenoh-router.md](../../0_doc/architecture/zenoh-router.md)

## 設計方針

| 層 | 役割 |
|:---|:---|
| protocol | 既存 `frame_key` / `input_key` / `manifest_key` / `presence_key` / `host_control_key` を正とする。`*_KEY_LOCAL` は `room_id=local` の別名として残す。`PROTOCOL_PIN` を `0.8.0-x0017` に。任意で `room_id` 検証ヘルパ（ASCII・`/` なし） |
| host | `ROOM_ID` から 5 キーを構築して declare。ログ・smoke banner も動的キーを出す。Room Token の `room_id` 検証は現状どおり（env と一致） |
| client | 同上。Manifest / Presence / frame / input / host_control すべて同一 `ROOM_ID` |
| Identity | 変更なし（Token `sub`） |

| Phase | 内容 | 状態 |
|:---|:---|:---|
| 0 | Slice 文書 + 目次 | done |
| 1 | protocol pin + `room_id` 検証（任意）+ 正本 docs | planned |
| 2 | host / client を `*_key(room_id)` に切替 | planned |
| 3 | smoke（別 `ROOM_ID` 非交差）+ README | planned |

## キー契約

- 式: `alchemy/dev/room/{room_id}/{frame|input|manifest|presence|host_control}`
- `ROOM_ID` 省略時は `local`（既存キーと互換）
- `room_id` 制約: 空でない・ASCII 印刷可能・`/` を含まない（不正なら起動失敗）
- host と client の `ROOM_ID` が一致しないと Manifest/Presence/frame が届かない（意図どおり）
- `PROTOCOL_PIN`: `alchemy-protocol/0.8.0-x0017`

## 保証する / しない

| 保証する | 保証しない |
|:---|:---|
| host/client が同一 `ROOM_ID` で 5 チャネルを共有 | 公式リレー ACL・テナント名前空間 `alchemy/tenant/...` |
| 異なる `ROOM_ID` 同士は Zenoh キーで隔離 | world-server の部屋一覧 UI・ディープリンク解決 |
| 既定 `local` は従来キーと同一文字列 | `room_id` の Unicode / URL encode 規則の完成形 |
| Room Token の `room_id` と env `ROOM_ID` の一致（既存） | トークン内 room と Zenoh キーの自動同期以外の発見プロトコル |

## やらない（後続）

- client アカウント殻（login / logout / 最小 register）→ **[x0018](x0018-client-auth-session.md)**
- 明示 Leave / liveliness
- Steal / 投げ / アバター GLB
- launcher / verifier / 予測補間

## 起動（同一マシン smoke）

```bash
zenohd -c zenoh-router/config.json5

# 部屋 A
ROOM_ID=alpha cargo run -p host-server
ROOM_ID=alpha ALCHEMY_FORCE_DESKTOP=1 AUTH_USER=alice cargo run -p client

# 部屋 B（別ターミナル・別 host）— alpha の frame/presence を受け取らないこと
ROOM_ID=beta cargo run -p host-server
ROOM_ID=beta ALCHEMY_FORCE_DESKTOP=1 AUTH_USER=bob cargo run -p client
```

既定（`ROOM_ID` 未設定）は従来どおり `local` で 1 host + N client。

## 実装メモ

- 置換対象: `client/app` / `client/host-server` の `declare_*(*_KEY_LOCAL)` と起動ログ
- smoke: `.scripts/smoke-presence-slots.sh` を pin / キー表示更新。任意で 2-room 非交差チェックを追加
- Dash Exit の `host_control` publish も `host_control_key(room_id)` に合わせる

← [目次](../README.md)
