#!/usr/bin/env bash
# x001a: HTTPS edge (Tunnel) — health ×3 + login → Room Token + assets health.
# Requires AUTH_URL / WORLD_URL / ASSETS_URL (https://… after Tunnel is up).
set -euo pipefail

usage() {
  cat <<'EOF'
usage: AUTH_URL=https://auth.example.tld \
       WORLD_URL=https://world.example.tld \
       ASSETS_URL=https://assets.example.tld \
       bash .scripts/smoke-https-edge.sh

Optional: AUTH_USER (default alice) AUTH_PASSWORD (default Password1) ROOM_ID (default local)
EOF
}

if [[ -z "${AUTH_URL:-}" || -z "${WORLD_URL:-}" || -z "${ASSETS_URL:-}" ]]; then
  usage
  exit 1
fi

AUTH_USER="${AUTH_USER:-alice}"
AUTH_PASSWORD="${AUTH_PASSWORD:-Password1}"
ROOM_ID="${ROOM_ID:-local}"

echo "smoke-https-edge: health checks…"
curl -sf --max-time 15 "${AUTH_URL}/health" >/dev/null
curl -sf --max-time 15 "${WORLD_URL}/health" >/dev/null
curl -sf --max-time 15 "${ASSETS_URL}/health" >/dev/null

echo "smoke-https-edge: login…"
LOGIN_JSON="$(curl -sf --max-time 30 -X POST "${AUTH_URL}/api/v1/auth/login" \
  -H 'content-type: application/json' \
  -d "{\"identifier\":\"${AUTH_USER}\",\"password\":\"${AUTH_PASSWORD}\",\"remember_me\":false}")"
ACCESS="$(printf '%s' "$LOGIN_JSON" | python3 -c 'import json,sys; print(json.load(sys.stdin)["access_token"])')"
if [[ -z "$ACCESS" ]]; then
  echo "smoke-https-edge: login did not return access_token"
  exit 1
fi

echo "smoke-https-edge: ensure room + Room Token…"
curl -sf --max-time 30 -X POST "${WORLD_URL}/api/v1/rooms" \
  -H 'content-type: application/json' \
  -H "authorization: Bearer ${ACCESS}" \
  -d "{\"room_id\":\"${ROOM_ID}\"}" >/dev/null || true

TOKEN_JSON="$(curl -sf --max-time 30 -X POST "${WORLD_URL}/api/v1/rooms/${ROOM_ID}/tokens" \
  -H 'content-type: application/json' \
  -H "authorization: Bearer ${ACCESS}" \
  -d '{"role":"host"}')"
ROOM_TOKEN="$(printf '%s' "$TOKEN_JSON" | python3 -c 'import json,sys; d=json.load(sys.stdin); print(d.get("token") or d.get("room_token") or "")')"
if [[ -z "$ROOM_TOKEN" ]]; then
  echo "smoke-https-edge: room token response missing token field: ${TOKEN_JSON}"
  exit 1
fi

echo "smoke-https-edge: assets health (CAS path optional)…"
curl -sf --max-time 15 "${ASSETS_URL}/health" >/dev/null
if [[ -n "${ALCHEMY_BLOB_BLAKE3:-}" ]]; then
  curl -sf --max-time 30 \
    -H "authorization: Bearer ${ACCESS}" \
    "${ASSETS_URL}/api/v1/blobs/${ALCHEMY_BLOB_BLAKE3}" \
    -o /dev/null
  echo "smoke-https-edge: ok (health + login→Room Token + CAS blob)"
else
  echo "smoke-https-edge: ok (health + login→Room Token + assets health)"
fi
