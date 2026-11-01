# x0067 — zenohd ACL: キー族 put 分離と host→guest 配信

> **ID:** `x0067` · **slug:** `zenohd-acl-put-family-host-guest` · **status:** `done` · **area:** `[zenoh, relay, docs]` · **depends_on:** `[x004e, x0066]`  
> **種別:** 調査 → 修正（第1壁 put 分離を非対称 flow + interest egress で復帰）  
> **検証:** WAN 2026-11-01（`relay.frick-eldy.com` / ConoHa / zenohd 1.10.1）

目標: **zenohd 1.10.1** で観測した「キー族 put 分離だと **host→guest（Manifest / Presence）が届かない**」現象の原因を特定し、第1壁の put 分離を **Guest sim の put ingress を解禁せず** 復帰する。

正本: [`config.wan.json5`](../../../relay-server/config.wan.json5) · [relay-server-wan.md](../../0_doc/manual/relay-server-wan.md) · [x0066](x0066-ephemeral-mtls-world-issue.md)。

## 保証する / しない（完了時）

| 保証する | 保証しない |
|:---|:---|
| キー族 put 分離（guest sim **put ingress** deny）+ host↔guest 双方向配信が WAN で通る | zenohd 本体のパッチ；全版・全トポロジの一般証明 |
| 非対称 ingress/egress + subscriber interest の egress 伝播が正本に残る | ホットリロード / per-room subject |
| guest の sim 偽装 Publish を transport で遮断（trace で ingress put unauthorized） | Billing / CCU QoS |

## 確定した原因（WAN trace 2026-11-01）

単なる「Guest に put egress が無い」だけでは不足だった。`388b768` 相当（guest put egress あり）でも host→guest は沈黙。interceptor trace で分かったこと:

1. **Put の入口／出口はキー族 × flow で非対称**  
   - Host→Guest: Host `put` **ingress** + Guest `put` **egress**（Guest `put` **ingress** は deny のまま）  
   - Guest→Host: Guest `put` **ingress** + Host `put` **egress**
2. **Subscriber interest の egress 伝播が必要**（zenohd 1.10.1）  
   - Host→Guest 配信: Host 面に Guest の `declare_subscriber` が **egress** で届くこと（`host-egress-declare`）。これ無しだと Guest 側 Put egress が authorized でも Manifest が届かない  
   - Guest→Host 連続 Intent: Guest 面に Host の `declare_subscriber` が **egress** で届くこと（`guest-egress-interest` on `input`/`alive`）。これ無しだと初回 Intent のみ → Presence Intent TTL（3s）で `reason=ttl` reclaim（「追い出された」）
3. Host sim の `put` も **ingress+egress**（`host-put-sim`）。ingress のみだと Host `Put (egress) on …/manifest` が unauthorized（実験 B）

**妥協しない点:** Guest の sim **`put` ingress** は許可しない（空間乗っ取り / Fan-out 増幅の遮断）。trace で `alchemy-guest … unauthorized to Put (ingress) on …/manifest` を確認済み。

デバッグ: `RUST_LOG=zenoh::net::routing::interceptor=trace`（検証後は systemd drop-in を外す）。

## 正本ルール（検証済み）

| rule id | subject | flows | messages（要点） | key_exprs（要点） |
|:---|:---|:---|:---|:---|
| `host-put-sim` | host | ingress+egress | put, delete, declare_queryable, reply, liveliness_token | sim + alive |
| `host-sub-input` | host | ingress | declare_subscriber, … | input / alive / sim |
| `host-egress-input` | host | egress | put, delete, liveliness_token, query, reply | input / alive / snapshot |
| `host-egress-declare` | host | egress | declare_subscriber, declare_liveliness_subscriber | input / alive / sim |
| `guest-put-input` | guest | ingress | put, delete, liveliness_token | input / alive only |
| `guest-sub-sim` | guest | ingress | declare_subscriber, … | sim + alive |
| `guest-egress-sim` | guest | egress | put, delete, liveliness_token, reply | sim + alive |
| `guest-egress-interest` | guest | egress | declare_subscriber, declare_liveliness_subscriber | input / alive |

`shared_memory.enabled: false`。

## 観測事実（2026-11-01）

| 構成 | guest→host | host→guest | 備考 |
|:---|:---|:---|:---|
| ingress のみ・キー族分離 | 失敗 | 失敗 | |
| egress 追加・キー族分離（≒`388b768`） | 成功（初回） | 失敗 | |
| egress・分離・SHM off | 成功（初回） | 失敗 | |
| `room/**` put 統合・SHM off | 成功 | 成功 | 回避策（第1壁弱体化） |
| + host sim put egress（実験 B） | 成功（初回） | 失敗 | Guest Put egress は既に authorized |
| + `host-egress-declare`（実験 C） | 初回のみ→TTL reclaim | **成功** | Manifest 到達；Intent が続かない |
| + `guest-egress-interest`（正本） | **成功（連続）** | **成功** | プレイ継続 OK |

## 完了条件

1. ~~原因と非対称 ACL が文書化~~ done  
2. ~~`config.wan.json5` / wan / trust-boundary が一致~~ done  
3. ~~zenohd 版・`388b768`・結論が Slice に残る~~ done  
4. ~~WAN: **正本8ルール一式**を載せた状態で配信 + プレイ継続 + guest sim put ingress 拒否（trace）~~ done（段階表の最終行；個別足しの途中段階だけでは完了としない）

## Phase

| Phase | 内容 | 状態 |
|:---|:---|:---|
| 0 | 課題 Slice + 観測固定 + 失敗 ACL 正本参照 | done |
| 1 | 原因調査（put egress + interest egress） | done |
| 2 | 正本 ACL 更新 + 保証表整合 | done |
| 3 | WAN 再検証（配信・連続 Intent・sim put ingress deny） | done |
| 4 | （任意）upstream / 新 zenohd 再検証 | planned |

## やらない

- Guest の sim **put ingress** を許可して配信を通すこと  
- `room/**` put 統合を恒久正本に戻すこと  
- JWT を zenohd に載せる / per-room 動的 ACL（別課題）

## 関連

- 正本: [`relay-server/config.wan.json5`](../../../relay-server/config.wan.json5)
- 失敗記録: `git show 388b768:relay-server/config.wan.json5`
- [relay-server-wan.md](../../0_doc/manual/relay-server-wan.md) · [trust-boundary.md](../../0_doc/architecture/trust-boundary.md)
- [x004e](x004e-relay-agent-mvp.md) · [x0066](x0066-ephemeral-mtls-world-issue.md)

← [目次](../README.md)
