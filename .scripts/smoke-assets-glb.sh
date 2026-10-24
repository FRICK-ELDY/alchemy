#!/usr/bin/env bash
# Slice C-B: upload Coin fixture → download → BLAKE3 match.
set -euo pipefail
ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
AUTH="${AUTH_URL:-http://127.0.0.1:4002}"
ASSETS="${ASSETS_URL:-http://127.0.0.1:4004}"
USER_NAME="${AUTH_USER:-alice}"
PASSWORD="${AUTH_PASSWORD:-Password1}"
FIXTURE="${ROOT}/client/fixtures/Coin_PolygonalMind.glb"
EXPECTED_BLAKE3="${ALCHEMY_BLOB_BLAKE3:-fa24848fa7b971718ce6e8dc9bd2d211808fd64e4ddf555feb66d4f82ee1e72a}"
OUT="$(mktemp)"

test -f "$FIXTURE"

echo "assets health:"
curl -sf "$ASSETS/health"
echo

echo "login:"
ACCESS=$(curl -sf -X POST "$AUTH/api/v1/auth/login" \
  -H 'content-type: application/json' \
  -d "{\"identifier\":\"${USER_NAME}\",\"password\":\"${PASSWORD}\"}" \
  | python3 -c 'import sys,json; print(json.load(sys.stdin)["access_token"])')
echo "access_token acquired (${#ACCESS} chars)"

echo "upload Coin_PolygonalMind.glb:"
RESP=$(curl -sf -X POST "$ASSETS/api/v1/blobs" \
  -H "authorization: Bearer ${ACCESS}" \
  -H 'content-type: model/gltf-binary' \
  -H 'X-Alchemy-Filename: Coin_PolygonalMind.glb' \
  --data-binary @"$FIXTURE")
echo "$RESP"
BLAKE3=$(printf '%s' "$RESP" | python3 -c 'import sys,json; print(json.load(sys.stdin)["blake3"])')
test "$BLAKE3" = "$EXPECTED_BLAKE3"

echo "download:"
curl -sf -H "authorization: Bearer ${ACCESS}" \
  "$ASSETS/api/v1/blobs/${BLAKE3}" -o "$OUT"
cmp -s "$FIXTURE" "$OUT"
echo "bytes match fixture"

if command -v b3sum >/dev/null 2>&1; then
  GOT=$(b3sum --no-names "$OUT")
  test "$GOT" = "$EXPECTED_BLAKE3"
  echo "b3sum ok"
fi

rm -f "$OUT"
echo "smoke-assets-glb: ok"
