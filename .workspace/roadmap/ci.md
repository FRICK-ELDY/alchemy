# ローカル CI

GitHub Actions と同じゲートを手元で実行する。**Elixir 系は全 OS で Docker 既定**。

```bash
make ci
make ci-rust
make ci-relay      # x0068 alchemy-relay-agent（client workspace 外）
make ci-protocol
make ci-fuzz       # x004c cargo-fuzz smoke（Linux/macOS + nightly；Windows は SKIP）
make ci-auth
make ci-world
make ci-assets
make ci-shared     # x0089 shared/alchemy_*（Docker 既定・deps/_build は named volume；SHARED_CI_HOST=1 でホスト mix）

# x001f — Hermetic Compose 縦 E2E（外ネット不要）
docker compose -f docker-compose.e2e.yml up -d --build
make ci-e2e
# または一発: COMPOSE_UP=1 make ci-e2e
docker compose -f docker-compose.e2e.yml down
```

Desktop 手通し（x0020）は日常 `:4002` 系 + zenohd の **手動 GUI**。機械検証の正本は上の `ci-e2e`（旧 `smoke-desktop-handthrough` 等の banner-grep は [x0089](slices/x0089-rotten-smoke-shared-ci.md) で削除）。

詳細は [README.md](../../README.md)、Slice [x001f](slices/x001f-hermetic-compose-e2e.md) / [x0020](slices/x0020-desktop-handthrough.md) / [x0089](slices/x0089-rotten-smoke-shared-ci.md)。

← [目次](README.md)
