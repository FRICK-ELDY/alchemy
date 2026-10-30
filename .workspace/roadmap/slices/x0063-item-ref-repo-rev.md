# x0063 — ItemRef `repo@rev`

> **ID:** `x0063` · **slug:** `item-ref-repo-rev` · **status:** `planned` · **area:** `[protocol, host, assets]` · **depends_on:** `[x0055, x0062]`  
> **ladder:** Wave 4 · [inventory-ladder.md](../inventory-ladder.md)

目標: ItemRef が **blake3** に加え **`repo_id` + `rev`（Commit または Ref 名）** を取れる。旧 blake3 パスは残す。

## 保証する / しない

| 保証する | 保証しない |
|:---|:---|
| ItemRef が blake3 \| repo@rev を表現できる | 全 Ref 名解決の UX |
| take-out が repo@rev でも Prefab/Tree 展開できる | 自動最新追従（常に main HEAD 強制） |
| 旧 blake3 のみの ItemRef が回帰緑 | PROTOCOL 非互換を黙って通す（載荷時は PIN bump） |
| 無権限 rev は失敗 | Origin Cert 検証実行 |

## 設計メモ

- ワイヤに載せるなら PROTOCOL_PIN bump を本 Slice の完了条件に含める

| Phase | 内容 | 状態 |
|:---|:---|:---|
| 0 | 本 Slice | planned |
| 1 | 型拡張 + 解決 + tests | planned |
| 2 | take-out 配線 + docs | planned |

## 完了条件

1. repo@rev と blake3 の両パスで take-out 可能
2. 不正 rev / 無 Grant は失敗

## やらない

- Branch 作成 UI（x0064）
- Shop

← [目次](../README.md) · [ladder](../inventory-ladder.md)