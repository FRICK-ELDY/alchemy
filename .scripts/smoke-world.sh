#!/usr/bin/env bash
set -euo pipefail
BASE="${WORLD_URL:-http://127.0.0.1:4003}"

echo "health:"
curl -sf "$BASE/health"
echo

echo "create room:"
curl -sf -X POST "$BASE/api/v1/rooms" \
  -H 'content-type: application/json' \
  -d '{"room_id":"local"}' || curl -s -X POST "$BASE/api/v1/rooms" \
  -H 'content-type: application/json' \
  -d '{"room_id":"local"}'
echo

echo "token:"
curl -sf -X POST "$BASE/api/v1/rooms/local/tokens" \
  -H 'content-type: application/json' \
  -d '{"role":"host","sub":"dev"}' | head -c 200
echo

echo "jwks:"
curl -sf "$BASE/.well-known/jwks.json" | head -c 120
echo
echo "smoke-world: ok"
