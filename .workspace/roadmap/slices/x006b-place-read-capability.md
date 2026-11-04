# x006b — PlaceRead（場所スコープ読取能力）

> **ID:** `x006b` · **slug:** `place-read-capability` · **status:** `done` · **area:** `[world, assets, docs]` · **depends_on:** `[x004f, x0039, x003a]`  
> **優先度:** BlobGrant 案 B の前提（発行者先行）

目標: **その room の `world-server` だけが** 短命 `PlaceRead` JWT を発行できる。中身の正本は **ManifestSeal 連動の `manifest_rev`**。`extra_blake3` は無効（take-out は別経路）。assets は検証・執行のみ（台帳に room_read を書かない）。

根拠: backlog P0-次（案 B）の発行境界 · 会話合意（場所で付与 / 急がば回れ）

## 保証する / しない

| 保証する | 保証しない |
|:---|:---|
| PlaceRead の `iss` は部屋の world（Room Token と同 JWKS）。`typ=place_read` / `aud=alchemy-assets-place-read` | GET 既定を常時 Grant 必須にする案 B 本体（後続 Slice） |
| 発行は有効 Room Token + 検証済み `world_seal`（ManifestSeal）必須。client 申告の blake3 一覧を無検証でサインしない | 連合 S2S 自動ミラー・他ノード assets への権利複製 |
| seal の `(room_id, revision, host_generation, manifest_blake3)` を PlaceRead に転写 | 退出即時のグローバル revoke（TTL + Token 失効に依存） |
| **ManifestSeal 発行時**に scene blake3 を PlaceHashCache へ固定。PlaceRead `exp` = **seal 絶対 unix `exp`**；発行時 `touch_until(seal.exp)`。再起動後は re-seal 必須 | JWT 内全 hash 列挙；durable 永続；canonical 再 hydrate；PlaceRead が seal より長生きすること |
| `extra_blake3` は **無効**（非空は 422）。take-out は所有／部屋内束縛付きの別経路まで待る | Inventory take-out UI / client 自動取得配線 |
| assets は `x-alchemy-place-read` 検証後、hash ∈ **seal-time scene** なら `grant_required` GET を許可。応答 identity 照合・verify 短命メモ化 | `authenticated` 既定の廃止；`extra_blake3` バイパス |

## 設計（要点）

```
host ─► world POST /manifest_seal  （このとき PlaceHashes 固定）
Client ─Room Token─► world POST /place_read { world_seal }
world ─PlaceRead JWT─► Client
Client ─Bearer + x-alchemy-place-read─► assets GET /blobs/:blake3
assets ─（miss 時）PlaceRead─► world GET /place_hashes
```

- 発行者 = 場所の world のみ
- 形態 = 短命 JWT（台帳行なし）
- hash 展開 = seal-time PlaceHashCache（TTL）+ assets 短命キャッシュ（identity 照合）
- `extra_blake3` = 無効（権限拡大防止）

## Phase

| Phase | 内容 | 状態 |
|:---|:---|:---|
| 0 | Slice + goal / backlog / README | done |
| 1 | world: ManifestSeal.verify + PlaceRead 発行 + place_hashes | done |
| 2 | assets: world JWKS + PlaceRead 門番 + hash キャッシュ | done |
| 3 | テスト + architecture 正本注記 | done |

## 完了条件

1. guest Room Token + 正当 `world_seal` → PlaceRead 200。偽 seal / 別 room → 拒否（422 / 403）
2. 非空 `extra_blake3` → 422 `extra_blake3_disabled`
3. assets: `grant_required` で Grant 無しでも PlaceRead カバー hash は 200。カバー外は 403
4. PlaceRead の `sub` と auth JWT `sub` 不一致 → 403
5. PlaceRead `exp` == seal `exp`；発行時 cache を seal.exp まで touch。期限切れ seal → `seal_expired`。再起動後 re-seal（409）
6. 保証表とコード一致。`mix test`（world / assets）緑

## やらない（本 Slice）

- client の自動 PlaceRead 取得・メッシュ DL 配線
- GET 既定の capability 廃止（案 B）
- `CasObject.access` の Grant/ref 移設（P1-1 本体）
- 新規 Hex 依存

← [目次](../README.md)
