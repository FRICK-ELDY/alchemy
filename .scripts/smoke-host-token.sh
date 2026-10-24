#!/usr/bin/env bash
# Verify host-server can fetch + verify a Room Token.
set -euo pipefail
ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT/client"

if [[ ! -x target/debug/host-server ]]; then
  cargo build -q -p host-server
fi

set +e
RUST_LOG=info timeout 5 ./target/debug/host-server > /tmp/host-smoke.log 2>&1
set -e

if grep -q "host Room Token acquired and verified" /tmp/host-smoke.log; then
  echo "smoke-host-token: ok"
  exit 0
fi

echo "smoke-host-token: FAILED"
cat /tmp/host-smoke.log
exit 1
