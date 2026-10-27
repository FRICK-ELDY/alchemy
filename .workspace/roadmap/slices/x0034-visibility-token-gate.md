# x0034 — Room visibility + Token 門番（private / friends / public）

> **ID:** `x0034` · **slug:** `visibility-token-gate` · **status:** `done` · **area:** `[world, auth, docs]` · **depends_on:** `[x0033]`

目標: Room に `visibility`。Token 発行前に ACL。friends は auth S2S。JWT に関係を載せない。

正本: [world-server.md](../../0_doc/architecture/world-server.md)

## 設計

| 層 | 内容 |
|:---|:---|
| Room | `visibility`（既定 `public`・デモ互換） |
| S2S | `GET /internal/v1/access` + `x-alchemy-internal-token` |
| Token | owner 常可 / private 非 owner 拒否 / friends → auth / public 可 |

## やらない

- Friends+ / Followers*（x0035）
- LAN（x0036）
- client UI ピッカー必須化

## 成功条件

1. friends Room に非フレンド guest → 403 `visibility_forbidden`
2. private は owner のみ Token
3. JWT claim に friend リストなし

← [目次](../README.md)
