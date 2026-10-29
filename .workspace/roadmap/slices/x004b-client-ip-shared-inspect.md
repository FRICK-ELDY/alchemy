# x004b — ClientIp 共有化・HTTP `inspect(reason)` 除去

> **ID:** `x004b` · **slug:** `client-ip-shared-inspect` · **status:** `done` · **area:** `[auth, world, assets, docs]` · **depends_on:** `[x002e, x002f]`  
> **優先度:** **P3-6**

目標: 信頼プロキシ ClientIp を **1 実装**にし、world/assets の JSON 応答から内部 `inspect(reason)` を外す。

根拠: [improvement-plan-2026-10-28.md](../../0_doc/evaluation/improvement-plan-2026-10-28.md) **P3-6**

## 保証する / しない

| 保証する | 保証しない |
|:---|:---|
| [`shared/alchemy_client_ip`](../../../shared/alchemy_client_ip/) が auth/world の ClientIp 正本（`otp_app` の `:trusted_proxies`） | Cloudflare Access / プロキシ自動検出 |
| `alchemy_auth_client` は JWKS/verify のみ（ClientIp・`remote_ip` を持たない） | assets への ClientIp 必須導入 |
| world `lan_eligible?` 等は薄いラッパ | Hex 公開 |
| world/assets のエラー JSON に `detail: inspect(...)` を載せない（分類コードのみ） | サーバログから `inspect` を全廃 |

## 実装メモ

- ClientIp を `alchemy_auth_client` に載せると auth が JWT 依存を引き、assets が `remote_ip` を lock する → **別 path パッケージ**に分離（レビュー対応）
- `plug` / `remote_ip` は `alchemy_client_ip` のみ（auth/world は既存利用の横展開）。人確認前提で追加済み

## 完了条件

- shared ClientIp 単体 + auth/world 委譲スモーク緑
- assets `mix.lock` に ClientIp 由来の `remote_ip` が増えない
- room/blob JSON に `detail: inspect` が残っていない

← [目次](../README.md)
