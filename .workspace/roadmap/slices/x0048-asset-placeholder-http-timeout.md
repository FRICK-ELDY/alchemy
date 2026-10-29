# x0048 — fixture fallback debug 限定 + HTTP timeout + error placeholder

> **ID:** `x0048` · **slug:** `asset-placeholder-http-timeout` · **status:** `done` · **area:** `[client, docs]` · **depends_on:** `[x003c]`  
> **優先度:** **P3-3**

目標: 認証済み assets GET / blake3 失敗を Coin fixture で隠さない。debug の `ALCHEMY_ALLOW_FIXTURE_FALLBACK` のみ fixture を許し、本番は **error placeholder mesh** + Home status。GET に connect/read timeout（hung 転送の cancel）を付ける。

根拠: [improvement-plan-2026-10-28.md](../../0_doc/evaluation/improvement-plan-2026-10-28.md) **P3-3**

正本: [client.md](../../0_doc/architecture/client.md)

## 保証する / しない

| 保証する | 保証しない |
|:---|:---|
| release: assets 失敗時は Coin fixture を使わない | live mesh swap / 再認証後の自動再取得 state machine |
| debug + `ALCHEMY_ALLOW_FIXTURE_FALLBACK` のみ fixture へ逃げる | cooperative cancel（HTTP timeout 以外） |
| assets GET connect 5s / read 30s（**read は idle／1 read 上限**。細切れ転送の壁時計上限ではない） | R2 / CDN 側の独自 deadline；cooperative mid-stream cancel |
| 失敗時は unit-cube placeholder + **sticky** Home status | placeholder の最終 UI アート；live mesh swap |
| 未認証 Home は Coin fixture 正本（Manifest blake3 不一致でも exit しない） | 未認証での Manifest mesh 取得 |

## 完了条件

- `cargo test -p client` 緑（`placeholder_error_mesh_is_not_empty_and_not_coin`）
- 認証失敗経路が process exit せず placeholder + status で起動継続
- 保証表とコード一致

← [目次](../README.md)
