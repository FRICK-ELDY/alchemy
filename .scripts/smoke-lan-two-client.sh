#!/usr/bin/env bash
# x0014 legacy smoke — superseded by smoke-presence-slots.sh (x0015).
# Still checks multi-slot units; host banner accepts x0015 (user_roots=3..6).
set -euo pipefail
ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT/client"

cargo build -q -p host-server
cargo test -q -p host-server -- \
  sim::tests::two_slots_move_independently \
  sim::tests::grab_no_steal_from_other_slot \
  --exact

AUTH_URL="${AUTH_URL:-http://127.0.0.1:4002}"
if ! curl -sf --max-time 2 "${AUTH_URL}/health" >/dev/null 2>&1; then
  echo "smoke-lan-two-client: ok (units only; auth not reachable — skip host run)"
  echo "  prefer: bash .scripts/smoke-presence-slots.sh"
  exit 0
fi

LOG="$(mktemp)"
set +e
RUST_LOG=info timeout 8 cargo run -q -p host-server >"$LOG" 2>&1
set -e

ok=1
if ! grep -qE "host-server x001[45] starting" "$LOG"; then
  echo "smoke-lan-two-client: missing host-server start banner"
  ok=0
fi
if ! grep -qE "user_roots=3\.\.[46]" "$LOG"; then
  echo "smoke-lan-two-client: missing user_roots=3..N in banner"
  ok=0
fi
if ! grep -q "protocol_pin=alchemy-protocol/" "$LOG"; then
  echo "smoke-lan-two-client: missing PROTOCOL_PIN"
  ok=0
fi
if ! grep -q "published SceneManifest revision=" "$LOG"; then
  echo "smoke-lan-two-client: missing SceneManifest publish"
  ok=0
fi
if ! grep -qE "zenoh connect=" "$LOG"; then
  echo "smoke-lan-two-client: missing zenoh connect log"
  ok=0
fi
if ! grep -q "physics: steps=" "$LOG"; then
  echo "smoke-lan-two-client: missing physics stats"
  ok=0
fi

if [[ "$ok" -ne 1 ]]; then
  echo "smoke-lan-two-client: FAILED"
  cat "$LOG"
  rm -f "$LOG"
  exit 1
fi

rm -f "$LOG"
echo "smoke-lan-two-client: ok (prefer smoke-presence-slots.sh for x0015)"
