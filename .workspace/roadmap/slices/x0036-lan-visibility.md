# x0036 — lan 公開範囲（外部拒否）

> **ID:** `x0036` · **slug:** `lan-visibility` · **status:** `done` · **area:** `[world, docs]` · **depends_on:** `[x0034]`

目標: `lan` は同一 LAN 上のみ。WAN / Cloudflare Tunnel 経由は Token 拒否（fail-closed）。

## 設計

| 判定 | 内容 |
|:---|:---|
| 可 | ClientIp が loopback / RFC1918 / link-local（`fe80::/10`）/ ULA（`fc00::/7`）、かつ CF Tunnel ヘッダなし |

| 否 | 公網 IP、または `cf-connecting-ip` / `cf-ray` あり |

関係グラフは見ない（認証済みなら LAN 上は入場可）。

## 成功条件

1. Tunnel ヘッダ付き → 403 `lan_required`
2. architecture 保証表に外部拒否を明記

← [目次](../README.md)
