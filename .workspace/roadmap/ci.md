# ローカル CI

GitHub Actions と同じゲートを手元で実行する。**Elixir 系は全 OS で Docker 既定**。

```bash
make ci
make ci-rust
make ci-launcher   # x0047 alchemy-launcher（client workspace 外）
make ci-protocol
make ci-fuzz       # x004c cargo-fuzz smoke（Linux/macOS + nightly；Windows は SKIP）
make ci-auth
make ci-world
make ci-assets

# x001f — Hermetic Compose 縦 E2E（外ネット不要）
docker compose -f docker-compose.e2e.yml up -d --build
make ci-e2e
# または一発: COMPOSE_UP=1 make ci-e2e
docker compose -f docker-compose.e2e.yml down

# x0020 — Desktop 手通し（日常 :4002 系 + zenohd :7447 が起動済みであること）
make smoke-desktop-handthrough
```

詳細は [README.md](../../README.md)、Slice [x001f](slices/x001f-hermetic-compose-e2e.md) / [x0020](slices/x0020-desktop-handthrough.md)。

← [目次](README.md)
