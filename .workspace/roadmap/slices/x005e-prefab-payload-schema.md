# x005e — Prefab ペイロードスキーマ（論理 Tree）

> **ID:** `x005e` · **slug:** `prefab-payload-schema` · **status:** `planned` · **area:** `[assets, protocol, docs]` · **depends_on:** `[x0055, x005d]`  
> **ladder:** Wave 3 · [inventory-ladder.md](../inventory-ladder.md)

目標: Prefab の中身を **論理 Tree**（`schema_version` / 子 `local_id` + `blake3` + ローカル TRS + 任意 Slot 名）として固定する。CAS 上は grant_required の 1 オブジェクトだが、**意味は opaque ではない**。

## 保証する / しない

| 保証する | 保証しない |
|:---|:---|
| Prefab 文書スキーマ（JSON または protobuf）と検証 | host での展開 spawn（→ [x005f](x005f-inventory-take-out.md)） |
| 子参照は blake3（mesh 等）。親 DriveNode は別 | VcsTree への自動変換実行（→ [x0062](x0062-prefab-promote-vcs.md)） |
| 不正 schema / 非 finite TRS を拒否 | マテリアル／スキン本格 |
| C+3 で同じ Tree が VcsTree になる旨を文書化 | 3D Diff UI |

## 設計メモ

- host は「展開して entity 列にする」だけ。Git 意味論は持たない
- 検証ライブラリの新規依存が必要なら **人に回す**（guardrails）

| Phase | 内容 | 状態 |
|:---|:---|:---|
| 0 | 本 Slice（スキーマ正本） | planned |
| 1 | 検証実装 + テスト | planned |
| 2 | architecture / draft への昇格メモ | planned |

## 完了条件

1. 合法 Prefab 受理・不正拒否のテスト
2. inventory PUT（x0054）経由で CAS 保存できる経路の文書または薄い helper

## やらない

- take-out / put-back の UX
- Origin Cert

← [目次](../README.md) · [ladder](../inventory-ladder.md)