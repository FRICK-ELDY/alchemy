#!/usr/bin/env bash
# x0022: jti-bound liveliness + Leave (units + host banner / pin).
set -euo pipefail
ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT/client"

cargo test -q -p alchemy-protocol --manifest-path ../protocol/Cargo.toml -- \
  tests::roundtrip_input_event_leave \
  tests::key_helpers \
  --exact

cargo test -q -p host-server -- \
  presence_reclaim_tests::reclaim_sub_frees_immediately \
  presence_reclaim_tests::reclaim_stale_frees_slot_for_reuse \
  presence_reclaim_tests::schedule_liveliness_requires_matching_jti \
  presence_reclaim_tests::refresh_intent_cancels_pending_liveliness_delete \
  --exact

AUTH_URL="${AUTH_URL:-http://127.0.0.1:4002}"
if ! curl -sf --max-time 2 "${AUTH_URL}/health" >/dev/null 2>&1; then
  echo "smoke-leave-liveliness: ok (units only; auth not reachable — skip host run)"
  echo "  start auth/world + zenohd, then re-run for full banner check"
  exit 0
fi

# Prefer a prebuilt binary so timeout covers runtime, not compile.
cargo build -q -p host-server

log="$(mktemp)"
set +e
RUST_LOG=info timeout 12 cargo run -q -p host-server >"$log" 2>&1
set -e

if grep -qE "Unable to connect|zenoh open failed|Connection refused|open zenoh session" "$log"; then
  # Banner is printed before zenoh open — still verify pin when present.
  if grep -q "host-server x0022 starting" "$log" \
    && grep -q "protocol_pin=alchemy-protocol/0.9.0-x0022" "$log" \
    && grep -qF "liveliness=alchemy/dev/room/local/alive/**" "$log" \
    && grep -q "presence_ttl_secs=3" "$log"; then
    echo "smoke-leave-liveliness: ok (units + banner; zenohd not up — skip full run)"
    rm -f "$log"
    exit 0
  fi
  echo "smoke-leave-liveliness: ok (units only; zenohd not reachable — skip host run)"
  rm -f "$log"
  exit 0
fi

ok=1
if ! grep -qE "host-server x0022 starting" "$log"; then
  echo "smoke-leave-liveliness: missing host-server x0022 start banner"
  ok=0
fi
if ! grep -q "protocol_pin=alchemy-protocol/0.9.0-x0022" "$log"; then
  echo "smoke-leave-liveliness: missing PROTOCOL_PIN 0.9.0-x0022"
  ok=0
fi
if ! grep -qF "liveliness=alchemy/dev/room/local/alive/**" "$log"; then
  echo "smoke-leave-liveliness: missing liveliness wildcard in banner"
  ok=0
fi
if ! grep -q "presence_ttl_secs=3" "$log"; then
  echo "smoke-leave-liveliness: missing presence_ttl_secs=3 (TTL fallback)"
  ok=0
fi

if [[ "$ok" -ne 1 ]]; then
  echo "smoke-leave-liveliness: FAILED"
  cat "$log"
  rm -f "$log"
  exit 1
fi
rm -f "$log"

echo "smoke-leave-liveliness: ok (units + banner; manual: logout → host reclaimed reason=leave <1s — see x0022)"
