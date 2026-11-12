# x008b — `/ready`（Repo・Keys・Port）+ blob GET octet-stream/nosniff

> **ID:** `x008b` · **slug:** `ready-and-blob-nosniff` · **status:** `done` · **area:** `[auth, world, assets, docs]` · **depends_on:** `[x0075]`  
> **優先度:** 評価 P2-2 / P2-3

目標: `/health` を liveness のまま残し、`/ready` で依存（Repo / Keys / Port）を検査する。assets local blob GET は CasObject.mime を Content-Type に出さず、常に `application/octet-stream` + `X-Content-Type-Options: nosniff` とする。

根拠: [improvement-plan-2026-11-05.md](../../0_doc/evaluation/improvement-plan-2026-11-05.md) **P2-2 / P2-3** · [opus-specific-weaknesses-2026-11-05.md](../../0_doc/evaluation/opus/opus-specific-weaknesses-2026-11-05.md)

---

## 保証する / しない

| 保証する | 保証しない |
|:---|:---|
| `GET /health` は依存非検査の liveness | `/health` が Port / DB 障害を検知すること |
| world `GET /ready`: Repo `SELECT 1` + Token.Keys JWKS + seal Port `ready?` | 並行ハッシュ健全性・auth JWKS 鮮度 |
| assets `GET /ready`: Repo + BLAKE3 Port `ready?` | CAS バックエンド到達性・auth JWKS |
| auth `GET /ready`: Repo + Token.Keys JWKS | メール / 外部 IdP |
| Port `ready?/0` は **GenServer.call しない**（`:persistent_term` + `Port.info`）。長時間 hash 中も偽陰性にならない | reopen 瞬間の一瞬 `false` |
| 失敗時 **503** + `checks` マップ（`ok` / `error`） | 部分 ready の段階的 200 |
| local blob GET Content-Type は常に `application/octet-stream` + `nosniff` | CasObject.mime 先勝ちの撤廃；S3 オブジェクト MIME 強制；表示用 mime のサーバー配信 |
| e2e compose は **auth のみ** `healthcheck: /ready`（Keys/Repo 待ちを依存に載せる）。world/assets の compose healthcheck は本 Slice では未追加 | 三サービスとも compose healthcheck 化 |

---

## 設計（要点）

| 軸 | 内容 |
|:---|:---|
| Ready モジュール | `Auth.Ready` / `World.Ready` / `Assets.Ready` — コントローラは JSON 整形のみ |
| Port ping | Worker が Port を `:persistent_term` に公開。`ready?/0` は `whereis` + `Port.info` のみ（hash 待ち行列に並ばない） |
| Keys 捕捉 | Auth Keys は raise、World Keys は call exit — Ready は `rescue` + `catch :exit` の両方 |
| mime | PUT 応答 / CasObject の mime はメタのまま。GET だけ無視。S3 302 の nosniff は中間ヘッダ用でオブジェクト MIME は非保証 |
| 公開面 | `/ready` は未認証（LB / compose 想定）。Tunnel 公網監視に使うかは運用判断（安価だが無制限 probe 可） |

---

## Phase

| Phase | 内容 | 状態 |
|:---|:---|:---|
| 0 | Slice + roadmap / improvement | **done** |
| 1 | Ready モジュール + `/ready` ルート + Worker ping | **done** |
| 2 | blob GET octet-stream + nosniff | **done** |
| 3 | テスト + architecture / threat-model | **done** |

---

## 完了条件

1. auth / world / assets で `GET /ready` が 200 + checks ok（正常起動時）
2. world `/health` は Port 非検査のまま
3. `text/html` で PUT した blob の local GET が `application/octet-stream` + `nosniff`
4. improvement P2-2 / P2-3 を done 表記

## やらない（本 Slice）

- mime を BlobRef へ移設
- Port プール化（P2-1）
- S3 側 Content-Type 書き換え
- Origin Cert content hash（P2-4）→ [x0096](x0096-origin-cert-content-hash.md)

## 関連

- [x0075-blake3-seal-hash-port.md](x0075-blake3-seal-hash-port.md)
- [improvement-plan-2026-11-05.md](../../0_doc/evaluation/improvement-plan-2026-11-05.md)

← [目次](../README.md)
