# x001f — Hermetic Compose 縦 E2E

> **ID:** `x001f` · **slug:** `hermetic-compose-e2e` · **status:** `done` · **area:** `[auth, world, assets, host, protocol, relay, ci, docs]` · **depends_on:** `[x001e]`

目標: 外界（Cloudflare Tunnel / R2 / ConoHa）に依存しない Docker Compose 上で、  
`login → Room Token → Zenoh InputEvent → RenderFrame` を機械的に緑にする。

根拠: [improvement-plan-2026-10-26.md](../../0_doc/evaluation/improvement-plan-2026-10-26.md) P3-7

## 用語

| 語 | 意味 |
|:---|:---|
| **Hermetic** | 密閉。Tunnel / R2 / VPS など外ネットに触れない。PC 内コンテナだけで完結 |
| **Compose** | ルート `docker-compose.e2e.yml` で auth / world / assets / zenohd を一式起動 |
| **E2E** | HTTP と Zenoh をまたぐ 1 本の経路を最後まで assert |

## 設計方針

1. ルート compose で共有ネットワーク。world/assets の `AUTH_JWKS_URL` は `http://auth:4002/...`
2. assets は **local CAS のみ**（`ASSETS_CAS_BACKEND=local`）
3. `e2e-probe` が login → guest Room Token → input put → frame 受信を assert
4. `host-server` は compose 外プロセス（smoke が起動）。GUI client は起動しない
5. サービス未起動・タイムアウトは **exit 1**（`units only` 成功禁止）

| Phase | 内容 | 状態 |
|:---|:---|:---|
| 0 | Slice 文書 + 目次 | done |
| 1 | `docker-compose.e2e.yml` + ヘルス待ち | done |
| 2 | `e2e-probe` バイナリ | done |
| 3 | `smoke-compose-e2e.sh` + `make ci-e2e` | done |
| 4 | GitHub Actions `e2e-compose.yml` | done |

## 保証する / しない

| 保証する | 保証しない |
|:---|:---|
| compose up 後 `make ci-e2e` が外ネット無しで緑 | Tunnel / WAN / R2 / Access |
| access JWT・Room Token・正当 input 後の RenderFrame 受信 | GUI client / OpenXR / 2-client 体感 |
| auth/world 停止時に smoke が赤 | Token refresh・JWKS bg・host role |
| 不正 Room Token の input 後も host が frame を出し続ける（生存） | 不正トークン拒否のメトリクス／ログ assert |
| zenohd イメージタグが client crate（`1.10.1`）と一致 | Zenoh ACL / TLS |
| auth `healthcheck` + world/assets `service_healthy` 依存 | Cloudflare / R2 |

## やらない（後続）

- x0020: 手元デモ縦（Desktop client 手通し）
- x0021: SceneManifest 駆動（定義 vs 実行）
- P1 全項目・WAN TLS・BlobGrant

## 成功条件

1. `docker compose -f docker-compose.e2e.yml up -d --build` 後、`make ci-e2e` が緑  
2. probe が RenderFrame（upserts 非空）を有限時間内に受信  
3. Cloudflare / R2 / ConoHa を参照しない  

## 起動メモ

ホスト公開ポート（開発用 `:4002` 系と共存）:

| サービス | ホスト | コンテナ内 |
|:---|:---|:---|
| auth | 4102 | 4002 |
| world | 4103 | 4003 |
| assets | 4104 | 4004 |
| zenohd | 7547 | 7447 |
| postgres | 5533 | 5432 |

```bash
docker compose -f docker-compose.e2e.yml up -d --build
make ci-e2e
# 片付け
docker compose -f docker-compose.e2e.yml down
```
