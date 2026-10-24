#!/usr/bin/env bash
set -euo pipefail
cd "$(dirname "$0")/../world-server"
docker compose run --rm --entrypoint "" -e MIX_ENV=test world \
  sh -c 'mix deps.get && mix format --check-formatted && mix compile --warnings-as-errors && mix credo --strict && mix test'
