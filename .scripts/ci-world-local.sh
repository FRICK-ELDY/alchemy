#!/usr/bin/env bash
# x003b: clear WORLD_DATABASE_PATH so developer compose `.env` cannot pollute ExUnit.
set -euo pipefail
cd "$(dirname "$0")/../world-server"
docker compose run --rm --entrypoint "" \
  -e MIX_ENV=test \
  -e WORLD_DATABASE_PATH= \
  world \
  sh -c 'mix deps.get && mix format --check-formatted && mix compile --warnings-as-errors && mix credo --strict && mix test'
