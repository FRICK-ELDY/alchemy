#!/usr/bin/env bash
set -euo pipefail
cd "$(dirname "$0")/../assets-server"
docker compose build assets
# x001e: force local CAS so developer .env (R2) cannot affect mix test
docker compose run --rm --entrypoint "" \
  -e MIX_ENV=test \
  -e ASSETS_CAS_BACKEND=local \
  assets \
  sh -c 'mix deps.get && mix format && mix format --check-formatted && mix compile --warnings-as-errors && mix credo --strict && mix test'
