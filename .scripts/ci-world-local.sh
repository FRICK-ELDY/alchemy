#!/usr/bin/env bash
# x003b: set a pollution WORLD_DATABASE_PATH; runtime.exs must ignore it in test.
set -euo pipefail
cd "$(dirname "$0")/../world-server"
docker compose run --rm --entrypoint "" \
  -e MIX_ENV=test \
  -e WORLD_DATABASE_PATH=/tmp/alchemy-world-test-must-ignore.db \
  world \
  sh -c 'mix deps.get && mix format --check-formatted && mix compile --warnings-as-errors && mix credo --strict && mix test'
