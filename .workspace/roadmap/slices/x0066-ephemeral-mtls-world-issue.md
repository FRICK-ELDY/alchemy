# x0066 — Ephemeral mTLS（world 発行）+ Zenoh ACL

> **ID:** `x0066` · **slug:** `ephemeral-mtls-world-issue` · **status:** `active` · **area:** `[world, relay, host, client, zenoh, docs]` · **depends_on:** `[x004e, x0022]`

目標: 静的 client 証明書の事前配布と「固定 IP allowlist 前提」をやめ、**クライアントがオンメモリで CSR を作り、world-server が私有 CA で短命 mTLS 証明書を署名**して WAN zenohd に入る。トピック汚染は **第1壁 Zenoh CN ACL**、部屋隔離は **第2壁 `wire_cap` キー**（現行のまま）。

## 保証する / しない

| 保証する | 保証しない |
|:---|:---|
| WAN: world が CSR を受け、**CSR Subject を無視**して role CN（`alchemy-host` / `alchemy-guest`）で短命 cert を署名 | QUIC 待受；`alchemy-core` 常時接続 |
| 第1壁: mTLS CN + `alchemy/dev/room/**` ACL（ingress/egress；WAN は SHM off） | キー族ごとの put 分離（暫定緩和・アプリ署名に寄る）；ACL ホットリロード / per-room subject |
| 第2壁: キー式 `alchemy/dev/room/{room_id}/{wire_cap}/…` 維持（x004e） | `alchemy/room/*` へのキー式移行 |
| ローカル `tcp/` と `RELAY_ALLOCATE_URL`（Relay Agent）は併存・回帰しない | Steam 公開面 Tunnel / IP allowlist 撤廃の運用完了 |
| client 秘密鍵を永続同梱しない（temp PEM は短命 0600） | 期限 30 分前の完全自動セッション再 open |
| Elixir 新規 Hex 依存なし（openssl CLI） | Billing / CCU QoS |

## 二重の壁

1. **第1壁 — Zenoh ACL（CN）**: 未認証 peer を弾き、`alchemy/dev/room/**` を role に許可（**egress 必須**）。キー族 put 分離は暫定緩和  
2. **第2壁 — `wire_cap`**: 部屋チャネル秘密 + Room Token / アプリ署名（既存）

## 完了条件

1. 未提示 / 偽 client cert → WAN zenohd が TLS 握手で拒否  
2. ~~guest CN で `…/frame` put 不可、`…/input` put 可~~ → **暫定緩和**（host→guest 配信のため。偽 sim はアプリ署名で discard。再強化は後続）  
3. CSR に偽 CN を書いても発行 CN はサーバー role 由来  
4. `csr_pem` なし・ローカル tcp は従来どおり  
5. Relay Agent 経路がまだ動く（world `mtls` 無し時）  
6. WAN: host 起動中に guest が Manifest を受信できる（egress + SHM off）

## Phase

| Phase | 内容 | 状態 |
|:---|:---|:---|
| 0 | Slice + 目次 + 設計書リダイレクト | done |
| 1 | `World.MtlsIssuer` + `create_token` `csr_pem` / `mtls` | done |
| 2 | WAN config コメント + 手順注記 | done |
| 3 | Rust `rcgen` + mtls 優先・allocate スキップ | done |
| 4 | テスト | done |

## 実装メモ

- Token 入室: `POST /api/v1/rooms/:room_id/tokens`（別 `/join` は作らない）
- CA: world（署名）と VPS zenohd `root_ca_certificate`（検証）で **同一 client CA**
- WAN 推奨: world `mtls` 応答。`RELAY_ALLOCATE_URL` は互換フォールバック

← [目次](../README.md)
