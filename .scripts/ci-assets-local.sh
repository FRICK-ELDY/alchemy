#!/usr/bin/env bash
set -euo pipefail
cd "$(dirname "$0")/../assets-server"
docker compose build assets
docker compose run --rm --entrypoint "" -e MIX_ENV=test assets \
  sh -c 'mix deps.get && mix format && mix format --check-formatted && mix compile --warnings-as-errors && mix credo --strict && mix test'
