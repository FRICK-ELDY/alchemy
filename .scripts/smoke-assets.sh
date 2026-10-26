#!/usr/bin/env bash
# Slice C-A: auth login → blob upload/download → quota → 401 without auth.
set -euo pipefail
AUTH="${AUTH_URL:-http://127.0.0.1:4002}"
ASSETS="${ASSETS_URL:-http://127.0.0.1:4004}"
USER_NAME="${AUTH_USER:-alice}"
PASSWORD="${AUTH_PASSWORD:-Password1}"
TMP="$(mktemp)"
OUT="$(mktemp)"
printf 'alchemy-smoke-payload' > "$TMP"

echo "assets health:"
curl -sf "$ASSETS/health"
echo

echo "login:"
ACCESS=$(curl -sf -X POST "$AUTH/api/v1/auth/login" \
  -H 'content-type: application/json' \
  -d "{\"identifier\":\"${USER_NAME}\",\"password\":\"${PASSWORD}\"}" \
  | python3 -c 'import sys,json; print(json.load(sys.stdin)["access_token"])')
echo "access_token acquired (${#ACCESS} chars)"

echo "upload:"
RESP=$(curl -sf -X POST "$ASSETS/api/v1/blobs" \
  -H "authorization: Bearer ${ACCESS}" \
  -H 'content-type: text/plain' \
  --data-binary @"$TMP")
echo "$RESP"
BLAKE3=$(printf '%s' "$RESP" | python3 -c 'import sys,json; print(json.load(sys.stdin)["blake3"])')

echo "download:"
# -L: follow 302 to R2 presigned URL when ASSETS_CAS_BACKEND=s3 (x001d)
curl -sfL -H "authorization: Bearer ${ACCESS}" \
  "$ASSETS/api/v1/blobs/${BLAKE3}" -o "$OUT"
cmp -s "$TMP" "$OUT"
echo "bytes match"

echo "quota:"
curl -sf -H "authorization: Bearer ${ACCESS}" "$ASSETS/api/v1/quota"
echo

echo "reject without auth:"
CODE=$(curl -s -o /dev/null -w '%{http_code}' -X POST "$ASSETS/api/v1/blobs" \
  -H 'content-type: text/plain' --data-binary @"$TMP")
test "$CODE" = "401"
echo "got HTTP $CODE (expected 401)"

rm -f "$TMP" "$OUT"
echo "smoke-assets: ok"
