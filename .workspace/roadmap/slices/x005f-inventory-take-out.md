# x005f — インベントリ取り出し（spawn）

> **ID:** `x005f` · **slug:** `inventory-take-out` · **status:** `planned` · **area:** `[host, assets, client]` · **depends_on:** `[x0057, x005e]`  
> **ladder:** Wave 3 · [inventory-ladder.md](../inventory-ladder.md)

目標: 所有者の Prefab／blob DriveNode から **ItemRef を解決し部屋に spawn** する。永続からの「取り出し」。

## 保証する / しない

| 保証する | 保証しない |
|:---|:---|
| Grant 保有者のみが対象 blake3 / Prefab を取り出せる | 他者インベントリの閲覧 |
| Prefab 展開 → entity 列 + 初期 Slot（held または世界） | put-back（→ [x0060](x0060-inventory-put-back.md)） |
| host は ItemRef / 展開結果だけを知る（Drive node_id を持たない／一時参照に閉じる） | `repo@rev`（→ [x0063](x0063-item-ref-repo-rev.md)） |
| 失敗時は部分 spawn を残さない（または明示ロールバック方針） | 空間内カメラ演出の完成 |

## 設計メモ

- client が assets から Prefab を取り、host に「定義に基づく実行」を渡す形を推奨（server=定義、client/host=実行の原則）
- ゲート詳細（誰が host に spawn 要求できるか）は Room Token role と整合

| Phase | 内容 | 状態 |
|:---|:---|:---|
| 0 | 本 Slice | planned |
| 1 | 展開 + spawn 経路 | planned |
| 2 | 否定系（無 Grant）+ docs | planned |

## 完了条件

1. owner が Prefab を部屋に出せる
2. 無 Grant は失敗
3. 既存 Grab / Manifest 回帰

## やらない

- 退出後の永続検証（→ [x0061](x0061-inventory-rejoin-roundtrip.md)）
- Vcs

← [目次](../README.md) · [ladder](../inventory-ladder.md)