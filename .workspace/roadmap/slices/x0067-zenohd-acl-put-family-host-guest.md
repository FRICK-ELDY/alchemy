# x0067 — zenohd ACL: キー族 put 分離と host→guest 配信

> **ID:** `x0067` · **slug:** `zenohd-acl-put-family-host-guest` · **status:** `planned` · **area:** `[zenoh, relay, docs, host, client]` · **depends_on:** `[x004e, x0066]`  
> **種別:** 調査課題（現行 WAN は回避策で運用）

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

## 仮説（未検証・優先順）

1. **egress put の subject / flow 組み合わせ**: guest 向け sim put が「egress 専用ルール」ではマッチせず、`flows: [ingress, egress]` 同一ルールに put を載せると通る（put 統合で偶然満たした）
2. **Interest / declare の伝播**: guest の `declare_subscriber` が host 面の egress を要し、現行 host ルール不足で publisher 側 interest が欠ける
3. **キー式 `*/*/` vs `**`**: inclusion マッチの差（input は `*/*/` で guest→host 成功しているため優先度低）
4. **ペイロードサイズ / 断片化**: Manifest 大・Presence 小の両方未到達なら優先度低
5. **zenohd 1.10.1 ACL 実装バグ**: 上記を潰したうえで upstream issue

## 調査手順（案）

1. ベース: WAN mTLS + `shared_memory.enabled: false` + LE listen（現行運用）
2. 最小差分だけ戻す（毎回 zenohd restart → host 起動維持 → silent client）:
   - A: 現行キー族分離のまま、`guest-egress-sim` と `guest-sub-sim` を **1 ルールにマージ**（`flows: [ingress, egress]`、messages に put + declare_subscriber）。guest の **sim put(ingress) はまだ付けない**
   - B: A がダメなら `host-put-sim` のみ `flows: [ingress, egress]`
   - C: guest sim キーに put を **egress のみ**残し、ingress put は input/alive のみ（現状）を維持したまま、host 側 interest 用に `declare_subscriber` egress を host へ追加
3. 各段で host ログ（assign / join assist）と client（Manifest blake3 / Presence）を記録
4. 通った最小構成を `config.wan.json5` 正本化し、本 Slice を done

## 完了条件

1. 上表に「最小で host→guest が通る ACL」または「分離復帰不可」の結論がある
2. `config.wan.json5` / wan 手順 / trust-boundary の第1壁記述が結論と一致
3. 可能なら zenohd 版・設定差分・再現ログへのリンクが Slice に残る
4. put 分離復帰時は「guest が sim に transport put できない」否定系を WAN または同等で 1 本

## Phase

| Phase | 内容 | 状態 |
|:---|:---|:---|
| 0 | 本課題 Slice + 観測の固定 | done |
| 1 | 最小差分 A/B/C の WAN 再現 | planned |
| 2 | 正本 ACL 更新 or 非保証の確定 | planned |
| 3 | （任意）upstream / 新 zenohd 再検証 | planned |

## やらない

- put 統合を「原因未特定のまま」再びキー族分離に戻すこと（回帰）
- ACL を緩めたまま「guest は frame put 不可」と保証表に書くこと
- JWT を zenohd に載せる / per-room 動的 ACL（別課題）

## 関連

- [`relay-server/config.wan.json5`](../../../relay-server/config.wan.json5)
- [relay-server-wan.md](../../0_doc/manual/relay-server-wan.md)
- [trust-boundary.md](../../0_doc/architecture/trust-boundary.md)
- [x004e](x004e-relay-agent-mvp.md) · [x0066](x0066-ephemeral-mtls-world-issue.md)

← [目次](../README.md)
