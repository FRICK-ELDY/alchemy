#!/usr/bin/env bash
# x001e: prod assets migrate via entrypoint (su-exec assets). Prefer this over bare `exec`.
set -euo pipefail
ROOT="$(cd "$(dirname "$0")/.." && pwd)"
cd "$ROOT/assets-server"
docker compose -f docker-compose.prod.yml --env-file .env.prod --profile migrate run --rm assets-migrate
