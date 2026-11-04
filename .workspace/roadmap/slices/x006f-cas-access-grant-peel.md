# x006f — CAS access → Grant/ref 移設

> **ID:** `x006f` · **slug:** `cas-access-grant-peel` · **status:** `done` · **area:** `[assets, docs]` · **depends_on:** `[x004f, x006c]`  
> **優先度:** backlog P1-1（列廃止・先着固定解消）

目標: `CasObject.access` 列を廃止し、読取能力の SSoT を **BlobGrant または PlaceRead** のみに固定する。PUT の `x-alchemy-blob-access`・先勝ち・409 `access_conflict` を撤去し、同一 blake3 の mesh / inventory 共存を保証する。

根拠: [backlog-priority-2026-11-04.md](../backlog-priority-2026-11-04.md) P1-1 · [assets-server.md](../../0_doc/architecture/assets-server.md) · improvement-plan P0-5

## 保証する / しない

| 保証する | 保証しない |
|:---|:---|
| `CasObject` は bytes メタのみ（blake3 / size / mime）。`access` 列なし | Grant 種別拡張（purchased / mail_temp） |
| GET は CasObject 存在 +（BlobGrant or PlaceRead）。未知 access 分岐なし | 存在秘匿（403 vs 404） |
| PUT は常に uploader へ `:owned` Grant。`x-alchemy-blob-access` はパースしない | ヘッダ互換レイヤの長期維持 |
| 同一 blake3 を `/blobs` と `/inventory/blobs` から別 owner が PUT しても **409 にならない** | mime/size 先勝ちの変更；orphan CAS GC |
| レスポンス JSON から `access` を削除 | client 新規 PUT 配線 |

## 設計（要点）

```
PUT → CasObject(size,mime) + BlobRef + owned BlobGrant
GET → CasObject あり → BlobGrant(auth_sub) OR PlaceRead covers → else 403
```

- 能力 = Grant の存在（または PlaceRead）。BlobGrant に `access` 列は足さない
- `/inventory/blobs` はパス契約として残す（x0054）。中身は通常 PUT と同じ
- mime/size 先勝ちは維持

## Phase

| Phase | 内容 | 状態 |
|:---|:---|:---|
| 0 | Slice + goal / backlog / README | done |
| 1 | migration + CasObject/BlobGrant schema | done |
| 2 | Blobs / Controller / Drive / Assets 保証表 | done |
| 3 | テスト（mesh+inventory 共存） | done |
| 4 | architecture 正本 + done | done |

## 完了条件

1. DB に `cas_objects.access` が無い
2. PUT が access ヘッダ無しで緑；同一 hash の mesh+inventory 共存テスト緑
3. GET は Grant/PlaceRead のみ（Bearer 単独 403 は維持）
4. `mix test`（assets-server）緑
5. 保証表・architecture・Slice が一致

## やらない（本 Slice）

- BlobGrant への `access` 列追加
- mime/size 先勝ちの変更、orphan reaper（P3）
- client PUT 実装、market / purchased Grant
- 新規 Hex 依存

← [目次](../README.md)
