# x001b — Zenoh WAN（ConoHa zenohd）

> **ID:** `x001b` · **slug:** `zenoh-wan-conoha` · **status:** `planned` · **area:** `[zenoh, host, client, docs, relay]` · **depends_on:** `[x001a]`

目標: ConoHa VPS（4Core / 4GB Ubuntu）上の **zenohd だけ**を公網 TCP で待受し、拠点 host + 外 client が `ZENOH_CONNECT` で同じ `ROOM_ID` に入る。x001a の HTTPS 入口と合わせて「外から入室できる」縦スライスを通す。

正本: [zenoh-router.md](../../0_doc/architecture/zenoh-router.md) · [overview.md](../../0_doc/architecture/overview.md)（公式リレー ConoHa）· [x001a](x001a-https-edge-tunnel.md)

## 設計方針

| 層 | 役割 |
|:---|:---|
| VPS zenohd | `tcp/0.0.0.0:7447`（または QUIC は後続）。**JWT は見ない** |
| 公開面 | firewall / セキュリティグループで **IP allowlist**（検証用）。全世界開放はしない |
| 拠点 host / client | `ZENOH_CONNECT=tcp/<VPS_IP>:7447`。host・auth/world/assets は拠点のまま |
| 外 client | x001a の `https://…` + 上記 `ZENOH_CONNECT` |

| Phase | 内容 | 状態 |
|:---|:---|:---|
| 0 | Slice 文書 + 目次 | done |
| 1 | VPS セットアップ runbook（zenohd インストール・systemd・firewall） | planned |
| 2 | `zenoh-router` WAN 用 config 例 + ローカルとの差分文書 | planned |
| 3 | 2 client（別ネット）Presence / Leave 動作確認 | planned |
| 4 | smoke 手順 + README / architecture 更新 | planned |

## 保証する / しない

| 保証する | 保証しない |
|:---|:---|
| VPS zenohd 経由で同一 `ROOM_ID` の pub/sub | Room Token / JWKS による接続 ACL（→ 後続） |
| 拠点 host + 外 client（または 2 外 client）の Presence / Leave | 拠点 zenohd ↔ VPS ブリッジ・マルチテナント隔離 |
| IP allowlist による公開面の最小化 | CCU / 帯域 QoS・Relay Allocation Token |
| x001a HTTPS と併用した外からの入室 runbook | QUIC / WebTransport / P2P |

## やらない（後続）

- エッジ JWT（Room Token → キー写像の薄い Relay Agent）
- Relay Allocation Token / 課金 QoS
- launcher / verifier / R2 / アバター GLB
- 公式リレーの本番 SLA 運用

## 成功条件（デモ）

1. x001a 済み（外から login / Room Token / assets 可）
2. VPS で zenohd 稼働、allowlist に拠点と検証クライアントのみ
3. 別ネットワークの 2 client が同じ room で互いの user_root が見える
4. logout → Leave で即時 reclaim（x0019 動作が WAN でも維持）

## 起動（予定）

```bash
# VPS
# zenohd -c /etc/alchemy/zenoh-wan.json5
# ufw: 7447/tcp from <allowlist only>

# 拠点 host（HTTPS は localhost で可）
export ZENOH_CONNECT=tcp/<VPS_IP>:7447
cd client && cargo run -p host-server

# 外 client（x001a URL + Zenoh）
export AUTH_URL=https://auth.example.tld
export WORLD_URL=https://world.example.tld
export ASSETS_URL=https://assets.example.tld
export ZENOH_CONNECT=tcp/<VPS_IP>:7447
ALCHEMY_FORCE_DESKTOP=1 cargo run -p client
```

## 実装メモ（方針）

- アプリのワイヤ契約は変えない。差し替えは `ZENOH_CONNECT` と到達 URL のみ
- 覗き見・ノイズは allowlist で抑える。ACL は次の縦（エッジ JWT）へ先送り
- レイテンシ体感をメモし、予測補間 Slice の判断材料にする

← [目次](../README.md)
