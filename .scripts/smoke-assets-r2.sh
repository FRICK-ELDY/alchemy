#!/usr/bin/env bash
# x001c/x001d: CAS backend=s3 (Cloudflare R2) — 302 presign check + API smoke.
# Requires assets-server already running with ASSETS_CAS_BACKEND=s3 and R2 credentials.
set -euo pipefail

usage() {
  cat <<'EOF'
usage: bash .scripts/smoke-assets-r2.sh

Requires assets-server with:
  ASSETS_CAS_BACKEND=s3
  ASSETS_S3_BUCKET / ASSETS_S3_ENDPOINT / ASSETS_S3_ACCESS_KEY_ID / ASSETS_S3_SECRET_ACCESS_KEY

Optional: AUTH_URL ASSETS_URL AUTH_USER AUTH_PASSWORD
See: .workspace/0_doc/manual/assets-cas-r2.md
EOF
}

if [[ "${1:-}" == "-h" || "${1:-}" == "--help" ]]; then
  usage
  exit 0
fi

AUTH="${AUTH_URL:-http://127.0.0.1:4002}"
ASSETS="${ASSETS_URL:-http://127.0.0.1:4004}"
USER_NAME="${AUTH_USER:-alice}"
PASSWORD="${AUTH_PASSWORD:-Password1}"
TMP="$(mktemp)"
printf 'alchemy-smoke-r2-presign' > "$TMP"

echo "smoke-assets-r2: login + upload…"
ACCESS=$(curl -sf -X POST "$AUTH/api/v1/auth/login" \
  -H 'content-type: application/json' \
  -d "{\"identifier\":\"${USER_NAME}\",\"password\":\"${PASSWORD}\"}" \
  | python3 -c 'import sys,json; print(json.load(sys.stdin)["access_token"])')

RESP=$(curl -sf -X POST "$ASSETS/api/v1/blobs" \
  -H "authorization: Bearer ${ACCESS}" \
  -H 'content-type: text/plain' \
  --data-binary @"$TMP")
BLAKE3=$(printf '%s' "$RESP" | python3 -c 'import sys,json; print(json.load(sys.stdin)["blake3"])')

echo "smoke-assets-r2: GET without -L must be 302 to R2…"
HEADERS=$(mktemp)
CODE=$(curl -s -D "$HEADERS" -o /dev/null -w '%{http_code}' \
  -H "authorization: Bearer ${ACCESS}" \
  "$ASSETS/api/v1/blobs/${BLAKE3}")
test "$CODE" = "302"
LOCATION=$(awk 'BEGIN{IGNORECASE=1} /^location:/ {sub(/\r$/,""); print $2; exit}' "$HEADERS")
test -n "$LOCATION"
echo "Location: $LOCATION"
case "$LOCATION" in
  https://*r2.cloudflarestorage.com/*) ;;
  *)
    echo "expected R2 host in Location, got: $LOCATION" >&2
    exit 1
    ;;
esac
case "$LOCATION" in
  *X-Amz-*) ;;
  *)
    echo "expected X-Amz- signature in Location" >&2
    exit 1
    ;;
esac

rm -f "$HEADERS" "$TMP"

echo "smoke-assets-r2: full API smoke with redirect follow…"
bash "$(dirname "$0")/smoke-assets.sh"
echo "smoke-assets-r2: ok (302 presign + API). Confirm object under prefix cas/ in R2 dashboard."
