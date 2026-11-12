# x0098 — take-out guest+session / owner_sub（遠隔 owner）

> **ID:** `x0098` · **slug:** `take-out-owner-session` · **status:** `done` · **area:** `[world, host, client, docs]` · **depends_on:** `[x0080, x0081]`  
> **優先度:** improvement **P2-7**（制御面 session 署名の残り：owner は guest Token のまま）

目標: inventory spawn/despawn の認可を **guest+session / `owner_sub`** 判定へ広げ、owner が別端末からでも計算ホストに take-out / despawn を依頼できるようにする。計算ホスト経路（`role=host` bind）は併存。Zenoh に Room Token を再導入しない（x0081 維持）。

根拠: [improvement-plan-2026-11-05.md](../../0_doc/evaluation/improvement-plan-2026-11-05.md) **P2-7** · [opus-specific-weaknesses-2026-11-05.md](../../0_doc/evaluation/opus/opus-specific-weaknesses-2026-11-05.md) · [x0081](x0081-control-plane-pop.md) 非保証「任意 guest の take-out」

---

## 保証する / しない

| 保証する | 保証しない |
|:---|:---|
| owner は guest Room Token + Intent bind のまま session 署名で spawn/despawn を送れ、計算ホストが受理する | 任意 guest / Invite ACL |
| 非 owner かつ非 host-role の guest bind は拒否 | owner と active host の同時矛盾編集の公平性 |
| Zenoh 平文に Room Token を再導入しない | client のローカル「自分は owner」推定だけで認可が完結すること |
| scene 書込は host プロセスの host Token + PoP（x0081） | PoP nonce / 単回使用 |
| 制御面 Dual: `(role=host ∧ sub==計算ホスト)` **または** `sub==owner_sub` | `owner_sub` 未取得時の owner 経路（fail-closed → host-role のみ） |

---

## 設計（要点）

### 制御面受理

```
evaluate_control:
  bound session
  AND control_seq advances
  AND (
    (role == "host" AND sub == active_host_token_sub)
    OR (sub == room.owner_sub)
  )
```

World HTTP append/delete は引き続き計算ホストの host Token + PoP + `ensure_active_host`。

### `owner_sub` 供給

- `GET …/scene` 応答に `owner_sub`（Room 行）を載せる
- host は scene 取得時に保持し、spawn/despawn evaluate に渡す

### client

- take-out / despawn UI から `host_room_token` 必須ゲートを外す
- Intent bind 時の `host_room_token` 優先は残す（非オーナー計算ホスト用）
- `install_host_inventory_token` は Host/Start/Rescue 用に残す（owner 遠隔の必須条件からは外す）
- **ソフトゲート**: カタログで他 owner が分かるときだけ早期拒否。`catalog_owner_sub == None`
  （Advanced 固定 `ROOM_ID` / カタログ未取得 / 行なし）は **早期拒否しない** — host Dual が最終判定

### host `owner_sub` 寿命

- 信頼ソースは **GET scene のみ**。append/delete 応答の `owner_sub` は無視
- `install_shared_scene_if_newer` は既知の `owner_sub` を汚染値で上書きしない
- **非保証**: ホスト再起動なしの ownership 移転反映（一度セットした `owner_sub` は以降の GET でも更新しない。要るなら GET 専用 refresh 分岐を別 Slice で）

---

## Phase

| Phase | 内容 | 状態 |
|:---|:---|:---|
| 0 | Slice + 目次 / improvement wiring | **done** |
| 1 | world: scene GET に `owner_sub` | **done** |
| 2 | host: 保持 + Dual evaluate_control + 単体否定系 | **done** |
| 3 | client: UI ゲート緩和 | **done** |
| 4 | docs / 保証表 / done | **done** |

---

## 完了条件

1. owner（guest JWT・`host_room_token` 無し）→ Intent bind → spawn **成功**
2. 別アカウント guest → spawn **reject**
3. takeover 後の active host（host-role bind）→ spawn **成功**
4. handover 後、別端末の owner（guest bind・`sub==owner_sub`）→ spawn **成功**（単体または統合で同等）
5. Zenoh payload に JWT が無い（x0081 回帰）
6. 保証表とコードが一致

**カバレッジ注記（実装時点）:** 1–4 の Dual 分岐は `SessionTable` 単体（guest+owner_sub / 非 owner reject / host-role non-owner / blank owner_sub fail-closed）と `install_shared_scene_if_newer` の owner_sub 保持で担保。Zenoh→world HTTP までの **手動 / WAN E2E は未実施**（別 PR または手元確認に残す）。

## やらない（本 Slice）

- Invite / 任意 guest 編集 ACL
- `host_room_token` 機構の完全削除
- 稼働中 host の `owner_sub` 再同期（ownership 移転なし再起動）
- bind 全体上限（P2-14）；XR loader tempfile は [x009a](x009a-xr-loader-tempfile-pure.md)

## 関連

- [x0080-host-handover-and-takeover.md](x0080-host-handover-and-takeover.md)
- [x0081-control-plane-pop.md](x0081-control-plane-pop.md)
- [x006d-host-take-out-control.md](x006d-host-take-out-control.md)
- [improvement-plan-2026-11-05.md](../../0_doc/evaluation/improvement-plan-2026-11-05.md)

← [目次](../README.md)
