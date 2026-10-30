# x0065 — Origin Certificate 薄皮

> **ID:** `x0065` · **slug:** `origin-cert-thin` · **status:** `planned` · **area:** `[assets, docs]` · **depends_on:** `[x0062, x0064]`  
> **ladder:** Wave 4 · [inventory-ladder.md](../inventory-ladder.md)

目標: Commit または Tag に紐づく **原産地署名の薄い版**（作者 `sub` + 対象ハッシュ + 署名）を置く。verifier 本実装・着用監査は別帯。

## 保証する / しない

| 保証する | 保証しない |
|:---|:---|
| Cert ペイロード検証（署名・subject・対象 Commit/Tag）の最小 | apps/verifier 本実装 |
| 改ざん Commit に対する Cert 不一致検知 | License Token / 購入証明 |
| 鍵管理方針が文書化される（秘密をログに出さない） | 連合・第三者 CA |
| C+3 到達宣言に必要な provenance の器 | Shop / market Webhook |

## 設計メモ

- 署名鍵の置き場・ローテは trust-boundary と整合。新規依存は人に回す
- 本 Slice で「薄い」＝発行と検証 API／ライブラリ最小。リアルタイム着用監査は world/verifier 後続

| Phase | 内容 | 状態 |
|:---|:---|:---|
| 0 | 本 Slice | planned |
| 1 | Cert スキーマ + 発行/検証 | planned |
| 2 | threat-model / draft 注記 · ladder C+3 | planned |

## 完了条件

1. 正規 Cert 検証 OK / 改ざん NG のテスト
2. 秘密鍵がレスポンス・ログに出ない
3. inventory-ladder 上で C+3 到達を注記可能

## やらない

- verifier プロセス
- Shopfront
- CRDT / Formula

← [目次](../README.md) · [ladder](../inventory-ladder.md)