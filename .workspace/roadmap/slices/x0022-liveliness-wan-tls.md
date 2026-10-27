# x0022 — liveliness セッション紐付け + WAN TLS

> **ID:** `x0022` · **slug:** `liveliness-wan-tls` · **status:** `done` · **area:** `[protocol, host, client, zenoh, docs]` · **depends_on:** `[x0021, x001b, x0019]`

目標: 2026-10-26 評価の **P2 前半**を塞ぐ。(1) Presence の `sub` だけで偽 liveliness Delete → 退室、を防ぐ。(2) WAN zenohd の平文 TCP 公開をやめ、TLS で暗号化する。

根拠: [improvement-plan-2026-10-26.md](../../0_doc/evaluation/improvement-plan-2026-10-26.md)（P2-1 / P2-3）

正本: [protocol.md](../../0_doc/architecture/protocol.md) · [zenoh-router.md](../../0_doc/architecture/zenoh-router.md) · [manual/zenoh-wan-conoha.md](../../0_doc/manual/zenoh-wan-conoha.md)

## 設計方針

| ID | 層 | 内容 |
|:---|:---|:---|
| P2-1 | protocol / host / client | liveliness キーを `…/alive/{sub}/{jti}` に変更。host は **検証済み Input で記録した jti と一致する Delete だけ** reclaim。未検証・不一致は無視（TTL フォールバック維持） |
| P2-3 | zenoh / docs / client·host | WAN `listen` を `tls/0.0.0.0:7447` のみ。証明書パスを config に明示。`ZENOH_CONNECT=tls/…` + 任意 `ZENOH_TLS_ROOT_CA`。ローカルは従来どおり `tcp` |

| Phase | 内容 | 状態 |
|:---|:---|:---|
| 0 | Slice 文書 + 目次 | done |
| 1 | P2-1 protocol pin + キーヘルパ + host 紐付け + client declare/refresh | done |
| 2 | P2-3 WAN TLS config + open_zenoh CA + 手順書 | done |
| 3 | architecture / smoke / 成功条件 | done |

## 契約

| 経路 | 信号 | host 動作 |
|:---|:---|:---|
| 検証済み Input | JWT verify 成功 | `sub → jti` 更新。**jti が変わったときだけ** pending Delete をキャンセル |
| 検証済み Leave | JWT verify 成功（`sub`） | **即** reclaim。jti 一致は不要（未失効の旧 Token でも Leave 可） |
| Liveliness Delete | `…/alive/{sub}/{jti}` | 記録 jti 一致なら **grace（1.5s）後**に reclaim。grace **内**に新 jti Intent が来ればキャンセル |
| Liveliness Delete（不一致・未記録） | 同上・偽キー | **無視**（ログ） |
| TTL | Intent 無音 3s | フォールバック（初回 Input 前のクラッシュ含む） |

- `PROTOCOL_PIN`: `alchemy-protocol/0.9.0-x0022`
- Token refresh（x0021）後: client は Intent put → 新 jti declare（`retain_previous=true` で旧キーを次 put まで保持）。join/re-login は `retain_previous=false`（二重キーなし）
- 正規切断の liveliness 反映は grace 分遅れる（同一 jti Intent では pending を消さない）

## 保証する / しない

| 保証する | 保証しない |
|:---|:---|
| Presence の `sub` alone では偽 Delete で退室できない | 盗聴した現行 Room Token の `jti` でのなりすまし（→ P2-2） |
| refresh 時 Delete(old)≺Intent(new) でも、Intent が **grace 内**に届けば誤 reclaim しない | grace **超過**の Intent 遅延・分断下での誤 reclaim |
| 同上 | 同一 jti の偽 Delete を Intent で即キャンセル（正規切断は常に +grace） |
| WAN 手順が平文 TCP 待受を指示しない（TLS 必須） | mTLS・Zenoh JWT ACL・Relay Agent・QUIC |
| ローカル / Compose E2E は従来 `tcp` のまま動く | Let's Encrypt 自動更新の運用自動化 |
| IP allowlist（ufw）をネットワーク ACL として維持 | テナント名前空間 ACL・Leave の jti 一致 / revoke feed |

## やらない（後続）

- session MAC / DPoP・毎フレーム Token 廃止（P2-2）
- client Frame 署名・sequence・snapshot（P2-4）
- PROTOCOL_PIN を Manifest/Presence に載せる（P2-5）
- SceneManifest 駆動（P4）

## 成功条件

1. protocol / host 単体: 不一致 jti は schedule しない；一致は grace 後 reclaim  
2. host 単体: Delete(old) schedule → Intent(new) → grace 後も reclaim しない（`refresh_intent_cancels_pending_liveliness_delete`）  
3. `config.wan.json5` が `tls/` のみ listen。手順書が証明書生成と `ZENOH_CONNECT=tls/…` を示す  
4. ローカル `make ci`（rust/protocol）緑。Compose E2E は tcp のまま通過  

## 起動メモ

```bash
# ローカル（従来）
zenohd -c zenoh-router/config.json5
ZENOH_CONNECT=tcp/127.0.0.1:7447 cargo run -p host-server

# WAN（詳細は manual）
# zenohd -c /etc/alchemy/zenoh-wan.json5   # tls + cert paths
# ZENOH_CONNECT=tls/<VPS_HOST>:7447
# ZENOH_TLS_ROOT_CA=/path/to/ca.pem        # 私有 CA のとき
```

← [目次](../README.md)
