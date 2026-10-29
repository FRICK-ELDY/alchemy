#!/usr/bin/env bash
# Slice B.5: auth login → Room create/token with Bearer.
set -euo pipefail
AUTH="${AUTH_URL:-http://127.0.0.1:4002}"
WORLD="${WORLD_URL:-http://127.0.0.1:4003}"
USER_NAME="${AUTH_USER:-alice}"
PASSWORD="${AUTH_PASSWORD:-Password1}"

echo "auth health:"
curl -sf "$AUTH/health"
echo

echo "login:"
ACCESS=$(curl -sf -X POST "$AUTH/api/v1/auth/login" \
  -H 'content-type: application/json' \
  -d "{\"identifier\":\"${USER_NAME}\",\"password\":\"${PASSWORD}\"}" \
  | python3 -c 'import sys,json; print(json.load(sys.stdin)["access_token"])')
echo "access_token acquired (${#ACCESS} chars)"

echo "create room:"
curl -sf -X POST "$WORLD/api/v1/rooms" \
  -H 'content-type: application/json' \
  -H "authorization: Bearer ${ACCESS}" \
  -d '{"room_id":"local"}' || curl -s -X POST "$WORLD/api/v1/rooms" \
  -H 'content-type: application/json' \
  -H "authorization: Bearer ${ACCESS}" \
  -d '{"room_id":"local"}'
echo

# x003e: session_pub required (32B base64url). Smoke uses a fixed dummy key.
SPK="$(printf '%s' 'AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA')"
# Prefer openssl if available for a random 32B; fall back to fixed.
if command -v openssl >/dev/null 2>&1; then
  SPK="$(openssl rand 32 | openssl base64 -A | tr '+/' '-_' | tr -d '=')"
fi
echo "token:"
curl -sf -X POST "$WORLD/api/v1/rooms/local/tokens" \
  -H 'content-type: application/json' \
  -H "authorization: Bearer ${ACCESS}" \
  -d "{\"role\":\"host\",\"session_pub\":\"${SPK}\"}" | head -c 200
echo

echo "reject without auth:"
CODE=$(curl -s -o /dev/null -w '%{http_code}' -X POST "$WORLD/api/v1/rooms" \
  -H 'content-type: application/json' \
  -d '{"room_id":"noauth"}')
test "$CODE" = "401"
echo "got HTTP $CODE (expected 401)"

echo "smoke-world: ok"
