# x0061 — 退出→再入室→取り出し（B+2 完了）

> **ID:** `x0061` · **slug:** `inventory-rejoin-roundtrip` · **status:** `planned` · **area:** `[host, assets, client, ci]` · **depends_on:** `[x005f, x0060]`  
> **ladder:** Wave 3 · [inventory-ladder.md](../inventory-ladder.md)

目標: **B+2 の定義完了** — put-back した Prefab が、退出・再入室のあと take-out で同じ論理内容として戻る。

## 保証する / しない

| 保証する | 保証しない |
|:---|:---|
| leave →（プロセス／部屋再参加）→ take-out で Prefab 子構成が一致 | ビット単位の entity_id 同一性 |
| Grant が再入室後も owner に残る | 他デバイス同時編集の競合解決（CRDT） |
| hermetic または手動手順のどちらかが正本として緑 | XR ホットスイッチ併記 |
| ladder 上で B+2 done と宣言できる | C+3（Vcs） |

## 設計メモ

- これが Wave 3 のゲート。ここまで緑になる前に Wave 4 に入らない

| Phase | 内容 | 状態 |
|:---|:---|:---|
| 0 | 本 Slice | planned |
| 1 | E2E または smoke 手順 | planned |
| 2 | ladder / architecture に B+2 到達を注記 | planned |

## 完了条件

1. 往復で Prefab 論理 Tree が一致（テストまたは記録済み手順）
2. 無 Grant ユーザは取り出せない

## やらない

- VcsRepo 昇格
- Shop

← [目次](../README.md) · [ladder](../inventory-ladder.md)