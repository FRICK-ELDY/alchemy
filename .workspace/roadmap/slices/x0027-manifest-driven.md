# x0027 — SceneManifest 駆動（P4 最小 / D2）

> **ID:** `x0027` · **slug:** `manifest-driven` · **status:** `done` · **area:** `[world, protocol, host, client, docs]` · **depends_on:** `[x0026]`

目標: 部屋の配置・loco・Grab 可否・mesh blake3 の正本を **world の Scene 定義 → sealed SceneManifest** に寄せる。host は Rapier 実行、client は定義に基づく描画/Grab。shared-sim / EntityKind は入れない。

正本: [protocol.md](../../0_doc/architecture/protocol.md) · [world-server.md](../../0_doc/architecture/world-server.md) · [client.md](../../0_doc/architecture/client.md)

## 設計方針

| ID | 層 | 内容 |
|:---|:---|:---|
| D2 | world | DemoScene を Room 作成時に紐付け。`GET …/scene` で host に配布 |
| D2 | host | scene entries から物理構築；Manifest を組み立て seal 公開 |
| D2 | client | mesh / grabbable を Manifest entries から（`10..18` 固定廃止） |
| — | — | floor コリジョンは host ローカルのまま |

## 契約

### SceneEntry（追加）

| フィールド | 意味 |
|:---|:---|
| `half_extent` | box 半辺。0 = 非 box |
| `flags` | `SCENE_FLAG_GRABBABLE = 1` |
| `r,g,b,a` | 初期色 |

解釈優先: (1) `USER_ROOT_*` → spawn (2) `blake3` 非空 → **mesh のみ** (3) `half_extent > 0` → box 描画/物理 (4) `flags & GRABBABLE` → Grab＝Dynamic、非 Grab＝Fixed。

### API

`GET /api/v1/rooms/:room_id/scene` — Bearer **host** Room Token → `{ revision, entries, locomotion }`。

### PIN

`alchemy-protocol/0.12.0-x0027`

## やらない

- shared-sim / EntityKind / UGC 編集 / Room DB 永続

## 成功条件

1. world DemoScene と一致する配置で部屋が立つ  
2. host の entries ハードコードが定義源にならない  
3. client Grab が Manifest flags 由来  
4. seal/attestation 維持；テスト緑  

← [目次](../README.md)
