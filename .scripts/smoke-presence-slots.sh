#!/usr/bin/env bash
# x0015: four user_root slots + Presence unit/banner when auth/world/zenohd are up.
set -euo pipefail
ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT/client"

cargo test -q -p alchemy-protocol --manifest-path ../protocol/Cargo.toml -- \
  tests::roundtrip_presence_state \
  tests::key_helpers \
  --exact

cargo build -q -p host-server
cargo test -q -p host-server -- \
  sim::tests::two_slots_move_independently \
  sim::tests::four_slots_have_distinct_spawns \
  sim::tests::grab_no_steal_from_other_slot \
  --exact

cargo test -q -p client -- \
  jwt_sub::tests::decodes_known_payload \
  --exact

AUTH_URL="${AUTH_URL:-http://127.0.0.1:4002}"
if ! curl -sf --max-time 2 "${AUTH_URL}/health" >/dev/null 2>&1; then
  echo "smoke-presence-slots: ok (units only; auth not reachable — skip host run)"
  echo "  start auth/world + zenohd, then re-run for full banner check"
  exit 0
fi

LOG="$(mktemp)"
set +e
RUST_LOG=info timeout 8 cargo run -q -p host-server >"$LOG" 2>&1
set -e

if grep -qE "Unable to connect|zenoh open failed|Connection refused" "$LOG"; then
  echo "smoke-presence-slots: ok (units only; zenohd not reachable — skip host run)"
  rm -f "$LOG"
  exit 0
fi

ok=1
if ! grep -qE "host-server x0015 starting" "$LOG"; then
  echo "smoke-presence-slots: missing host-server x0015 start banner"
  ok=0
fi
if ! grep -q "user_roots=3..6" "$LOG"; then
  echo "smoke-presence-slots: missing user_roots=3..6 in banner"
  ok=0
fi
if ! grep -q "presence=alchemy/dev/room/local/presence" "$LOG"; then
  echo "smoke-presence-slots: missing presence key in banner"
  ok=0
fi
if ! grep -q "protocol_pin=alchemy-protocol/0.6.0-x0015" "$LOG"; then
  echo "smoke-presence-slots: missing PROTOCOL_PIN 0.6.0-x0015"
  ok=0
fi
if ! grep -q "published PresenceState revision=0" "$LOG"; then
  echo "smoke-presence-slots: missing initial PresenceState publish"
  ok=0
fi
if ! grep -q "published SceneManifest revision=" "$LOG"; then
  echo "smoke-presence-slots: missing SceneManifest publish"
  ok=0
fi
if ! grep -q "physics: steps=" "$LOG"; then
  echo "smoke-presence-slots: missing physics stats"
  ok=0
fi

if [[ "$ok" -ne 1 ]]; then
  echo "smoke-presence-slots: FAILED"
  cat "$LOG"
  rm -f "$LOG"
  exit 1
fi

rm -f "$LOG"
echo "smoke-presence-slots: ok (manual: alice/bob/carol clients — see x0015 slice)"
