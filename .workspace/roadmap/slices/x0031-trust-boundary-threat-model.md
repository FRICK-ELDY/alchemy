# x0031 — trust-boundary threat model（P4-3）

> **ID:** `x0031` · **slug:** `trust-boundary-threat-model` · **status:** `done` · **area:** `[docs, zenoh, host, client]` · **depends_on:** `[x0022, x0023, x0030]`

目標: 未認証 publisher が各 Zenoh / HTTP 経路に到達したときの **防御責任**を architecture 正本に図示・表形式で固定する。

正本: [trust-boundary.md](../../0_doc/architecture/trust-boundary.md)

## 成果物

- `.workspace/0_doc/architecture/trust-boundary.md`（新規）
- overview / protocol / zenoh-router からリンク
- zenoh-router MVP キー表に manifest / presence / alive / snapshot / host_control を追記

## やらない

- mTLS / Relay Agent 実装
- BlobGrant 実装
- コードの検証ロジック変更（文書のみ）

## 成功条件

1. threat model 正本が architecture にある
2. キー表がコードの `*_key` ヘルパと一致
3. overview から辿れる

← [目次](../README.md)
