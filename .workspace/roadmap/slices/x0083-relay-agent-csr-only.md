# x0083 — relay-agent CSR-only（秘密鍵をネットワークに出さない）

> **ID:** `x0083` · **slug:** `relay-agent-csr-only` · **status:** `done` · **area:** `[relay, client, docs]` · **depends_on:** `[x004e, x0066, x0068]`  
> **優先度:** improvement P0-4

目標: 互換 Relay Agent の allocate を **world と同じ CSR 署名契約** に揃え、応答から `key_pem` を消し、非 loopback cleartext bind を起動拒否する。

根拠: [improvement-plan-2026-11-05.md](../../0_doc/evaluation/improvement-plan-2026-11-05.md) **P0-4** · [x0066](x0066-ephemeral-mtls-world-issue.md)（正経路）· [x004e](x004e-relay-agent-mvp.md)

---

## 保証する / しない

| 保証する | 保証しない |
|:---|:---|
| `POST /v1/allocate` は `csr_pem` 必須。CSR Subject 無視・role CN 強制（`-subj`） | Agent 退役；billing / 第三者リレー |
| 応答に **秘密鍵無し**（`key_pem` 削除）。client が鍵を保持 | TLS 終端付き非 loopback 待受の組み込み |
| `RELAY_AGENT_LISTEN` が loopback 以外なら **起動拒否** | CCU / QoS / L7 Zenoh proxy |
| zenoh-connect allocate は ephemeral CSR を送り、ローカル鍵で PEM を書く | world 正経路の廃止（推奨は引き続き world mtls） |

## 設計（要点）

```
client ──allocation_token + csr_pem──► relay-agent
relay-agent ──JWKS verify + openssl -subj /CN=role──► cert_pem + ca_pem
client ──local private key + cert_pem──► ZENOH_TLS_CONNECT_*
```

- 非 loopback（例: `0.0.0.0:7450`）は bail。WAN 到達は Tunnel / SSH で loopback に寄せる
- 手順書から cleartext 公網 bind 指示を外す

## Phase

| Phase | 内容 | 状態 |
|:---|:---|:---|
| 0 | Slice + 完了条件 | done |
| 1 | relay-agent CSR 署名・`key_pem` 削除・loopback 拒否 | done |
| 2 | zenoh-connect allocate 追随 | done |
| 3 | README / WAN 手順書 / backlog | done |
| 4 | `cargo test -p alchemy-relay-agent` / zenoh-connect | done |

## 完了条件

1. allocate 応答 JSON に `key_pem` が無い
2. `csr_pem` 欠落・空文字 → 400 `csr_pem_required`；署名は role CN 強制。bind 後も `local_addr` が loopback でなければ起動拒否
3. `RELAY_AGENT_LISTEN=0.0.0.0:…` で起動失敗
4. 手順書が CSR-only + loopback のみ
5. relay-agent / zenoh-connect テスト緑

## やらない（本 Slice）

- allocate API 自体の廃止（退役は別判断）
- JWKS curl 置換 / slowloris 全面対策（評価残課題）
- 新規 Cargo 依存

## 関連

- [x0066](x0066-ephemeral-mtls-world-issue.md) · [x0068](x0068-relay-mtls-ttl-bound-ci.md) · [relay-agent/README.md](../../../relay-agent/README.md)

← [目次](../README.md)
