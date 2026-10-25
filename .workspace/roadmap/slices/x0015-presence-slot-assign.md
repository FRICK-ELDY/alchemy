# x0015 — Presence 動的スロット割当（3人以上）

> **ID:** `x0015` · **slug:** `presence-slot-assign` · **status:** active · **area:** `[host, client, protocol, docs]` · **surface:** `[desktop, vr]` · **depends_on:** `[x0014]`

目標: client の **`AUTH_USER`→entity 固定マップを廃止**し、host が Room Token の **`sub` → user_root entity** を first-come で割り当て・公開する。client は自 `sub` で自 entity を発見し、カメラ追従する。同時接続は **最大 4**（entity `3..6`）。3 client（alice / bob / carol）smoke まで。

正本: [client.md](../../0_doc/architecture/client.md) · [protocol.md](../../0_doc/architecture/protocol.md)

## 設計方針

| 層 | 役割 |
|:---|:---|
| protocol | 低頻度 `PresenceState`（`sub`→`entity_id` 一覧）+ `PRESENCE_KEY_LOCAL`。`PROTOCOL_PIN` 更新 |
| host | 既存 first-come を **4 スロット**に拡張。割当変更時（と late-joiner 用に周期）に Presence を publish。Intent/Grab はスロット別のまま |
| client | Room Token から `sub` を読む → Presence で `local_user_root` を解決。未解決時は Intent 送信可・カメラは待機。他 root は色付きボックス（最大 3 remote） |
| Identity | 引き続き Token `sub` のみ（`InputEvent` に client_id なし） |

| Phase | 内容 | 状態 |
|:---|:---|:---|
| 0 | Slice 文書 + 目次 | done |
| 1 | protocol: `PresenceState` + key ヘルパ + pin | planned |
| 2 | host: `USER_ROOT_SLOTS=4`・spawn・Presence publish | planned |
| 3 | client: `sub` 解決 + Presence 購読 + 固定マップ削除 + remote 描画 | planned |
| 4 | seeds 確認（carol）+ smoke 3-client + README / 正本 | planned |

## エンティティ規約（本 Slice）

| 範囲 | 用途 |
|:---|:---|
| `1` | Coin |
| `2` | static（床など） |
| **`3..6`** | **user_root**（スロット 0..3） |
| `10..18` | grabbable boxes（変更なし） |

`AUTH_USER` は **ログイン用**のみ。entity 番号との対応は持たない。接続順でスロットが決まる（alice 先でも entity 3 とは限らないが、空室なら通常 3）。

## Presence 契約（案）

- Zenoh key: `alchemy/dev/room/local/presence`（`PRESENCE_KEY_LOCAL` / 将来 `presence_key(room_id)`）
- ペイロード: `PresenceState { revision, assignments: [{ sub, entity_id }] }`（occupied のみ）
- host: 割当成功時に publish。既存 Manifest 同様、late-joiner 向けに低頻度 republish 可
- client: 自 Room Token の `sub` と一致する `entity_id` を `local_user_root` にする

## 保証する / しない

| 保証する | 保証しない |
|:---|:---|
| 最大 4 `sub` まで first-come で user_root を割当・再利用なし（切断解放はしない） | 5 人目の動的スケール・UUID 汎用発見プロトコルの完成形 |
| Presence 経由で client が自 entity を知る（alice/bob 固定マップ廃止） | JWT 署名検証を client が行うこと（payload の `sub` 読み取りのみ） |
| 3 client（alice/bob/carol）が同時に移動・互いに見える | アバター GLB / 音声 / 予測補間 |
| 各 occupied スロットの Intent/Grab 分離（他 slot 保持中は無視） | Steal / 投げ / 切断時のスロット回収 |

## やらない（後続）

- 切断・トークン失効時のスロット解放
- Steal / 投げ / アバター GLB
- `ROOM_ID` → Zenoh キー本配線
- launcher / verifier / 予測補間

## 起動（同一マシン smoke・3 client）

```bash
zenohd -c zenoh-router/config.json5
cd client && cargo run -p host-server

ALCHEMY_FORCE_DESKTOP=1 AUTH_USER=alice cargo run -p client
ALCHEMY_FORCE_DESKTOP=1 AUTH_USER=bob cargo run -p client
ALCHEMY_FORCE_DESKTOP=1 AUTH_USER=carol cargo run -p client
```

## 実装メモ（予定）

- `USER_ROOT_SLOTS: 2 → 4`、`USER_SPAWN` を 4 点に拡張（boxes `10..` と衝突しない）
- client remote 描画: 自以外の occupied root をボックス表示（UBO スロット不足なら dedicated buffer）
- Room Token JWT の payload から `sub`（認証済み発行済み前提・検証は host 側）
- smoke: `.scripts/smoke-lan-two-client.sh` を拡張、または `smoke-presence-slots.sh`

← [目次](../README.md)
