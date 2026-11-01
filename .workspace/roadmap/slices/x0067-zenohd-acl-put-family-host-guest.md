# x0067 — zenohd ACL: キー族 put 分離と host→guest 配信

> **ID:** `x0067` · **slug:** `zenohd-acl-put-family-host-guest` · **status:** `planned` · **area:** `[zenoh, relay, docs]` · **depends_on:** `[x004e, x0066]`  
> **種別:** 調査課題（現行 WAN は回避策で運用）  
> **注:** Phase 1 の否定系観測に host/client ログを使うが、本 Slice 自体は relay ACL 調査が主目的。

目標: **zenohd 1.10.1** で観測した「キー族 put 分離 + egress だと guest→host は通るが **host→guest（Manifest / Presence）が届かない**」現象の原因を特定し、第1壁の put 分離を安全に復帰できる条件を決める。

根拠: WAN 手通し 2026-11-01（`relay.frick-eldy.com` / ConoHa）。運用回避は [`config.wan.json5`](../../../relay-server/config.wan.json5) の `room/**` put 統合 + SHM off。正本注記: [relay-server-wan.md](../../0_doc/manual/relay-server-wan.md) · [x0066](x0066-ephemeral-mtls-world-issue.md)。

## 保証する / しない（本 Slice 完了時）

| 保証する | 保証しない |
|:---|:---|
| 再現手順と最小 ACL 差分表が文書化される | zenohd 本体のパッチ提供 |
| host→guest が通る **最小** ACL（put 分離をどこまで戻せるか）が検証される | 全 Zenoh 版・全トポロジでの一般証明 |
| 復帰可能なら `config.wan.json5` を put 分離に戻し、保証表を更新 | ホットリロード / per-room subject |
| 復帰不能なら upstream 制約として保証表に残し、アプリ署名依存を明示 | Billing / CCU QoS |

## 観測事実（2026-11-01）

| 構成 | guest→host（Intent / liveliness / assign） | host→guest（Manifest / Presence） |
|:---|:---|:---|
| ingress のみ（egress なし）・キー族分離 | 失敗（`assignments=0`） | 失敗 |
| **egress 追加**・キー族分離・SHM 既定 | 成功 | 失敗 |
| egress・キー族分離・**SHM off** | 成功 | **失敗**（再確認） |
| egress・**`alchemy/dev/room/**` put 統合**・SHM off | 成功 | **成功** |

補足:

- guest→host 成功時: host ログに `presence: assigned` / `input gate: accepted` / `presence join assist` の Manifest 再送が出る
- host→guest 失敗時: client は `declare_subscriber` 成功後も sample 無し（`drop SceneManifest` も無し）→ **ACL/経路で未到達**（検証 reject ではない）
- 待ち時間延長（2s→8s）では解消しなかった（再送は届いていない）

### 失敗構成の正本（Phase 0）

**commit:** [`388b768`](https://github.com/FRICK-ELDY/alchemy/commit/388b768)（`fix(x0066): WAN ACL の put 分離を戻し Docker CA マウントを直す`）。  
作業ツリーの現行 `config.wan.json5`（統合版）とは別物。中身は次のみ:

```bash
git show 388b768:relay-server/config.wan.json5
```

SHM off + LE listen のうえで VPS に載せ、host→guest 失敗を再確認した版。

| rule id | flows | messages（要点） | key_exprs（要点） |
|:---|:---|:---|:---|
| `host-put-sim` | ingress | put, delete, declare_queryable, reply, liveliness_token | `…/*/*/manifest\|frame\|presence\|snapshot\|host_control\|alive/**` |
| `host-sub-input` | ingress | declare_subscriber, declare_liveliness_subscriber, liveliness_query, query | input / alive / sim keys |
| `host-egress-input` | egress | put, delete, liveliness_token, query, reply | input / alive / snapshot |
| `guest-put-input` | ingress | put, delete, liveliness_token | input / alive only |
| `guest-sub-sim` | ingress | declare_subscriber, … | sim keys + alive |
| `guest-egress-sim` | egress | put, delete, liveliness_token, reply | sim keys + alive |

policies: host ← 上3、guest ← 下3。`shared_memory.enabled: false`。

## 仮説（未検証・優先順）

1. **egress 専用 put ルールの効き方**: 失敗構成は guest sim put を **egress 専用**ルールにしている。同一ルールに `flows: [ingress,egress]` + `put` を載せると通る可能性はあるが、そのとき **ingress put も許可される**（＝put 分離が崩れる）。切り分け必須
2. **Interest / declare の伝播**: guest の `declare_subscriber` が host 面の egress を要し、現行 host ルール不足で publisher 側 interest が欠ける
3. **キー式 `*/*/` vs `**`**: inclusion マッチの差（input は `*/*/` で guest→host 成功しているため優先度低）
4. **ペイロードサイズ / 断片化**: Manifest 大・Presence 小の両方未到達なら優先度低
5. **zenohd 1.10.1 ACL 実装バグ**: 上記を潰したうえで upstream issue

## 調査手順（案）

ベース: WAN mTLS + `shared_memory.enabled: false` + LE listen。毎回 zenohd restart → host 起動維持 → silent client。

各段の成功判定（両方見る）:

- **配信:** client が Manifest blake3 / Presence を受信
- **分離維持（transport 層）:** guest から sim キー（例: `…/manifest`）への **ingress put が zenohd ACL で拒否**される（put API エラー / ルータ側 drop。アプリ署名 discard は数えない）。A2 以外でここが崩れたら「分離復帰」とは数えない

差分（失敗構成 `388b768` からの最小変更）:

| 段 | 内容 | ねらい | 解釈上の注意 |
|:---|:---|:---|:---|
| **A0（対照）** | `388b768` のまま再掲 | ベースライン再現 | guest-sub-sim と guest-egress-sim は **別ルールのまま** |
| **A1** | A0 を維持。ルール id / 文言以外は変えない再確認 | フレーク排除 | — |
| **A2（明示実験）** | guest の sim 向けに **1 ルール**へマージ: `flows: [ingress, egress]`、messages に `put` + `declare_subscriber` 等 | 「同一ルール化すると配信が通るか」 | **ingress put も許可される**前提。通っても「flow マージ」と「guest sim ingress put 解禁」が同時に起きる → 仮説1の肯定にはなるが **分離復帰ではない** |
| **B** | A0 ベースで `host-put-sim` のみ `flows: [ingress, egress]`（guest 側は分離のまま） | host 面 put の両 flow | guest sim ingress put は引き続き拒否であること |
| **C** | A0 ベースで host に sim/input 向け `declare_subscriber` **egress** allow を追加 | interest 仮説 | 同上。Manifest 受信だけで他要因と混ぜない |

通った最小構成のうち、**分離維持（guest sim ingress put 拒否）を満たすもの**があれば正本候補。A2 のみ成功なら「統合に近づいただけ」と記録し、分離復帰は未達とする。

## 完了条件

1. 上表に「最小で host→guest が通る ACL」または「分離復帰不可」の結論がある
2. `config.wan.json5` / wan 手順 / trust-boundary の第1壁記述が結論と一致
3. zenohd 版・失敗構成 commit（`388b768`）・各段の差分が Slice に残る
4. put 分離復帰時は「guest が sim に **ingress** put できない」否定系を WAN または同等で 1 本（**transport 拒否**を観測。アプリ discard のみは不可。各実験段でも同時観測）

## Phase

| Phase | 内容 | 状態 |
|:---|:---|:---|
| 0 | 本課題 Slice + 観測の固定 + 失敗 ACL 正本参照 | done |
| 1 | 最小差分 A0/A1/A2/B/C の WAN 再現 | planned |
| 2 | 正本 ACL 更新 or 非保証の確定 | planned |
| 3 | （任意）upstream / 新 zenohd 再検証 | planned |

## やらない

- put 統合を「原因未特定のまま」再びキー族分離に戻すこと（回帰）
- ACL を緩めたまま「guest は frame put 不可」と保証表に書くこと
- JWT を zenohd に載せる / per-room 動的 ACL（別課題）

## 関連

- 現行（回避）: [`relay-server/config.wan.json5`](../../../relay-server/config.wan.json5)（`e2778dd` 以降の `host-room` / `guest-room`）
- 失敗（分離）: `git show 388b768:relay-server/config.wan.json5`
- [relay-server-wan.md](../../0_doc/manual/relay-server-wan.md)
- [trust-boundary.md](../../0_doc/architecture/trust-boundary.md)
- [x004e](x004e-relay-agent-mvp.md) · [x0066](x0066-ephemeral-mtls-world-issue.md)

← [目次](../README.md)
