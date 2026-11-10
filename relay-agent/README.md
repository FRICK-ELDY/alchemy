# alchemy-relay-agent（x004e / x0068 / P0-4）

軽量 Relay Agent: **Allocation Token + client CSR → 短命 mTLS client 証明書（鍵は返さない）**。

| | 役割 |
|:---|:---|
| **relay-server** | zenohd の設定・待受（空間同期の仲介）→ [`relay-server/`](../relay-server/) |
| **relay-agent** | Allocation Token 検証 → CSR 署名のみ（本ディレクトリ） |

WAN 推奨経路は world 発行 mTLS（[x0066](../.workspace/roadmap/slices/x0066-ephemeral-mtls-world-issue.md)）。本 Agent は互換フォールバック（契約は world と同じ CSR-only）。

## 保証 / 非保証

| 保証 | 非保証 |
|:---|:---|
| `POST /v1/allocate` で `aud=alchemy-relay` を world JWKS 検証（soft + **hard TTL**） | CCU / Mbps QoS |
| **`csr_pem` 必須**（欠落 / `null` / 空白 → 400 `csr_pem_required`）・CSR Subject 無視・CN=`alchemy-host` / `alchemy-guest` 強制（openssl `-subj`） | L7 Zenoh プロキシ |
| 応答に **秘密鍵無し**（`key_pem` 廃止）。`expires_in=300` と **NotAfter ≈ 300s** | マルチリレー発見 |
| 非 loopback cleartext bind は **起動拒否**（`127.0.0.1` / `localhost` / `::1` のみ） | TLS 終端付き非 loopback 待受 |
| Body ≤ 16 KiB、allocate 同時 ≤ 8（health 除外）、接続 ≤ 32、`jti` は issue 前に reserve（失敗時 release）、**per-kid** unknown-kid cooldown（上限 256・満杯時 FIFO 追い出し） | プロセス全体 cooldown |
| `GET /health`；`make ci-relay` / GitHub Actions | |

## 環境変数

| 変数 | 既定 | 意味 |
|:---|:---|:---|
| `RELAY_AGENT_LISTEN` | `127.0.0.1:7450` | bind（**loopback 必須**） |
| `WORLD_JWKS_URL` | `http://127.0.0.1:4003/.well-known/jwks.json` | world JWKS |
| `RELAY_ID` | `alchemy-official` | Token `relay_id` 一致 |
| `RELAY_TLS_DIR` | `/etc/alchemy/tls` | `client-ca-cert.pem` / `client-ca-key.pem` |
| `RELAY_AGENT_WORK_DIR` | `/tmp/alchemy-relay-agent` | 一時 CSR / 署名作業 |
| `OPENSSL_BIN` | `openssl` | openssl パス（OpenSSL 3+ / `-not_after`） |

## 起動

```bash
# 証明書: relay-server/scripts/gen-wan-dev-certs.sh …（client-ca-* を含む）
cargo run --release
curl -fsS http://127.0.0.1:7450/health
```

host / client: `RELAY_ALLOCATE_URL=http://127.0.0.1:7450`（Zenoh open 前に allocate）。world が `mtls` を返した接続は allocate しない。公網から Agent に届ける場合は SSH/Tunnel 等で **loopback に届ける**（`0.0.0.0` cleartext は起動しない）。

## CI

```bash
make ci-relay
# または ./.scripts/ci.sh relay
```
