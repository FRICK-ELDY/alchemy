# x0089 — 腐った smoke 削除 + `ci.sh` に `shared/alchemy_*`

> **ID:** `x0089` · **slug:** `rotten-smoke-shared-ci` · **status:** `done` · **area:** `[ci, docs, host, client]` · **depends_on:** `[x0073, x001f, x0071]`  
> **優先度:** 評価 P2-8 / P2-9（衛生）

目標: CI に入らず現行ログ／`PROTOCOL_PIN` と乖離した smoke を削除し、機械検証は `e2e-probe`（`make ci-e2e`）と workspace `cargo test` に寄せる。あわせて `ci.sh all` に `shared/alchemy_*` 3 パッケージを入れ、ローカル CI と GitHub Actions の単一入口を揃える。

根拠: [improvement-plan-2026-11-05.md](../../0_doc/evaluation/improvement-plan-2026-11-05.md) **P2-8 / P2-9** · [opus-specific-weaknesses-2026-11-05.md](../../0_doc/evaluation/opus/opus-specific-weaknesses-2026-11-05.md)

---

## 保証する / しない

| 保証する | 保証しない |
|:---|:---|
| 腐った banner-grep smoke はリポジトリから削除され、README / Makefile から到達不能 | 残存 smoke（assets / compose / prod 等）の全件 CI 必須化 |
| host Token / Manifest / Leave / presence の機械検証は `make ci-e2e`（e2e-probe）と `cargo test` が正本 | Desktop GUI 手通しそのものの自動化 |
| `ci.sh shared` / `all` が `alchemy_auth_client` / `alchemy_client_ip` / `alchemy_rate_limit` の format / compile / test を回す（Docker 既定） | 共有パッケージの Hex 公開・credo 必須化 |
| Docker 経路はソース bind + **named volume で `deps`/`_build` を隠す**（auth/world/assets と同型）。ホスト製 BEAM をコンテナが拾わない | ホスト `SHARED_CI_HOST=1` と Docker 経路の成果物を同一ディレクトリで共有すること |

---

## 削除する smoke（吸収先）

| 削除 | 理由 | 吸収先 |
|:---|:---|:---|
| `smoke-desktop-handthrough.sh` | 旧 Room Token 文言 grep（現行ログと不一致） | `smoke-compose-e2e.sh` / e2e-probe |
| `smoke-desktop-noclip.sh` | 旧 `host-server x0009..` banner | `cargo test -p host-server`（noclip 等） |
| `smoke-lan-two-client.sh` | 旧 banner・vacuous success | 同上 + e2e-probe |
| `smoke-leave-liveliness.sh` | pin `0.9.0-x0022`・vacuous success | `cargo test` presence_reclaim / protocol Leave |
| `smoke-presence-slots.sh` | 同上クラスの pin/banner 腐敗 | `cargo test` presence / protocol keys |

---

## Phase

| Phase | 内容 | 状態 |
|:---|:---|:---|
| 0 | Slice + roadmap / improvement | **done** |
| 1 | 腐った smoke 削除 + README / Makefile / 旧 Slice 参照更新 | **done** |
| 2 | `ci.sh` + Makefile に `shared` | **done** |
| 3 | Docker named volume で deps/_build 隔離 + host `FAILED` 伝播修正 | **done** |

### Docker shared（設計メモ）

- 意図: **ホストの `deps`/`_build` を共有しない**。`shared/` を bind し、`alchemy-ci-shared-<pkg>-{deps,build}` named volume で上書きする
- hex/rebar は `alchemy-ci-shared-{mix,hex}` に永続化し、3 パッケージを **1 コンテナ**で回す（apk / `mix local` は 1 回）
- `SHARED_CI_HOST=1` はホスト側 `shared/*/deps` を使う別経路。Docker 既定と混ぜない

---

## 完了条件

1. 上記 5 スクリプトが無く、Makefile / README が `ci-e2e` を機械検証の入口にする
2. `./.scripts/ci.sh shared`（および `all`）が 3 パッケージを実行する
3. improvement-plan P2-8 / P2-9 を done 表記
4. Docker 経路でホスト製 `deps`/`_build` が影にならない（named volume）

← [目次](../README.md)
