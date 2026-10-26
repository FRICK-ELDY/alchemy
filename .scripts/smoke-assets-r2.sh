#!/usr/bin/env bash
# x001c: CAS backend=s3 (Cloudflare R2) — same API smoke as smoke-assets.sh.
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

echo "smoke-assets-r2: running API smoke (server must already use ASSETS_CAS_BACKEND=s3)…"
bash "$(dirname "$0")/smoke-assets.sh"
echo "smoke-assets-r2: ok (API). Confirm object under prefix cas/ in R2 dashboard."
