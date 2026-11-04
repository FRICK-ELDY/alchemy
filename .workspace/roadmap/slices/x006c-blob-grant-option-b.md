# x006c — BlobGrant 案 B（GET 常時 Grant / PlaceRead）

> **ID:** `x006c` · **slug:** `blob-grant-option-b` · **status:** `done` · **area:** `[assets, client, docs]` · **depends_on:** `[x004f, x006b]`  
> **優先度:** backlog P0-次（PlaceRead 発行境界の後続）

目標: GET 可否の SSoT を **常に** `BlobGrant` またはカバーする **PlaceRead** にする。`CasObject.access=authenticated` の認証済み capability を廃止する。部屋メッシュは x006b PlaceRead で維持する。

根拠: [backlog-priority-2026-11-04.md](../backlog-priority-2026-11-04.md) P0-次 · [assets-server.md](../../0_doc/architecture/assets-server.md) 案 B · x004f 移行段の目標形

## 保証する / しない

| 保証する | 保証しない |
|:---|:---|
| `GET /api/v1/blobs/:blake3` は Bearer +（**BlobGrant** または **PlaceRead カバー**）必須。`access=authenticated` でも capability 単独では読めない | `CasObject.access` 列の廃止・Grant/ref 本移設（P1-1 残り） |
| 既知 `access`（`authenticated` / `grant_required`）以外は引き続き fail-closed 403 | 存在秘匿（403 vs 404 オラクル） |
| 部屋メッシュ: client が Room Token + 検証済み `world_seal` で PlaceRead を取得し、`x-alchemy-place-read` 付き GET | PlaceRead 自動更新の完璧な TTL リフレッシュ UI；take-out `extra_blake3` |
| uploader（`:owned` Grant）は PlaceRead 無しでも自 hash を GET 可 | PUT 既定を `grant_required` へ強制；inventory UI |
| 保証表・architecture・テストが案 B と一致 | 新規 Hex 依存；market / License Token |

## 設計（要点）

```
GET authorize:
  CasObject あり
  → access ∈ {authenticated, grant_required}
  → BlobGrant(auth_sub, blake3) OR PlaceRead covers blake3
  → else 403
  → unknown access → 403
```

- PUT / `CasObject.access` 先勝ちは現状維持（メタ・共存 peel）
- GET は access 値で capability を分けない（両方とも Grant/PlaceRead）
- client: Manifest 受理時に `world_seal` を保持 → PlaceRead 発行 → assets GET

## Phase

| Phase | 内容 | 状態 |
|:---|:---|:---|
| 0 | Slice + goal クリア + 目次 / backlog | done |
| 1 | assets: `Blobs.fetch` 常時 Grant/PlaceRead + テスト | done |
| 2 | client: PlaceRead 取得 + `x-alchemy-place-read` 付きメッシュ DL | done |
| 3 | architecture / trust-boundary / backlog / README | done |

## 完了条件

1. `access=authenticated` でも Grant 無し・PlaceRead 無しの他 user GET → 403
2. 同条件で PlaceRead カバー → 200；カバー外 → 403
3. owner Grant → PlaceRead 無しで 200
4. client 入室メッシュ経路が PlaceRead ヘッダを付与（Room Token + seal があるとき）
5. `mix test`（assets）緑。保証表とコード一致

## やらない（本 Slice）

- `CasObject.access` 列削除 / access→Grant 本移設
- host take-out UI / Intent
- PUT 既定の `grant_required` 強制
- PlaceRead JWT のクライアント側期限前リフレッシュ専用 UI

← [目次](../README.md)
