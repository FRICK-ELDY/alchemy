# alchemy-relay-agent（x004e）

軽量 Relay Agent: **Allocation Token → 短命 mTLS client 証明書**。

## 保証 / 非保証

| 保証 | 非保証 |
|:---|:---|
| `POST /v1/allocate` で `aud=alchemy-relay` を world JWKS 検証 | CCU / Mbps QoS |
| CN=`alchemy-host` / `alchemy-guest` の cert（openssl CLI） | L7 Zenoh プロキシ |
| `expires_in=300`（再 allocate SLA）; openssl NotAfter ≈ 1 day | 秒単位の NotAfter |
| `GET /health` | マルチリレー発見 / 公網 cleartext の安全性 |

## 環境変数

| 変数 | 既定 | 意味 |
|:---|:---|:---|
| `RELAY_AGENT_LISTEN` | `127.0.0.1:7450` | bind |
| `WORLD_JWKS_URL` | `http://127.0.0.1:4003/.well-known/jwks.json` | world JWKS |
| `RELAY_ID` | `alchemy-official` | Token `relay_id` 一致 |
| `RELAY_TLS_DIR` | `/etc/alchemy/tls` | `client-ca-cert.pem` / `client-ca-key.pem` |
| `RELAY_AGENT_WORK_DIR` | `/tmp/alchemy-relay-agent` | 一時 CSR |
| `OPENSSL_BIN` | `openssl` | openssl パス |

## 起動

```bash
# 証明書: zenoh-router/scripts/gen-wan-dev-certs.sh …（client-ca-* を含む）
cargo run --release
curl -fsS http://127.0.0.1:7450/health
```

host / client: `RELAY_ALLOCATE_URL=http://…:7450`（Zenoh open 前に allocate）。
