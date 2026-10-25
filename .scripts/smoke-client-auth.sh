#!/usr/bin/env bash
# x0018: client auth session shell (login / logout / pin units).
set -euo pipefail
ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT/client"

cargo test -q -p client -- \
  auth::tests::auth_gate_clear_resets_join \
  system_ui::tests::home_tab_has_submit_and_bottom_tab \
  system_ui::dash::tests::open_home_tab_has_submit \
  --exact

AUTH_URL="${AUTH_URL:-http://127.0.0.1:4002}"
if ! curl -sf --max-time 2 "${AUTH_URL}/health" >/dev/null 2>&1; then
  echo "smoke-client-auth: ok (units only; auth-server not reachable)"
  exit 0
fi

LOGIN_JSON="$(curl -sf --max-time 5 -X POST "${AUTH_URL}/api/v1/auth/login" \
  -H 'content-type: application/json' \
  -d '{"identifier":"alice","password":"Password1","remember_me":false}')"
ACCESS="$(printf '%s' "$LOGIN_JSON" | python3 -c 'import json,sys; print(json.load(sys.stdin)["access_token"])')"
if [[ -z "$ACCESS" ]]; then
  echo "smoke-client-auth: login did not return access_token"
  exit 1
fi

curl -sf --max-time 5 -X POST "${AUTH_URL}/api/v1/auth/logout" \
  -H 'content-type: application/json' \
  -H "authorization: Bearer ${ACCESS}" \
  -d '{}' >/dev/null

echo "smoke-client-auth: ok (login→logout HTTP + Home tab units)"
