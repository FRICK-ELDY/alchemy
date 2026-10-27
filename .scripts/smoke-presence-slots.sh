#!/usr/bin/env bash
# Presence slots + ROOM_ID keys (banner / pin; x0019 pin supersedes x0017).
set -euo pipefail
ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT/client"

cargo test -q -p alchemy-protocol --manifest-path ../protocol/Cargo.toml -- \
  tests::roundtrip_presence_state \
  tests::key_helpers \
  tests::validate_room_id_accepts_local_and_rejects_bad \
  --exact

cargo build -q -p host-server
cargo test -q -p host-server -- \
  sim::tests::two_slots_move_independently \
  sim::tests::four_slots_have_distinct_spawns \
  sim::tests::grab_no_steal_from_other_slot \
  sim::tests::release_slot_clears_grab_and_resets_spawn \
  presence_reclaim_tests::reclaim_stale_frees_slot_for_reuse \
  presence_reclaim_tests::same_sub_before_ttl_keeps_slot \
  presence_reclaim_tests::reclaim_sub_frees_immediately \
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

check_host_banner() {
  local room="$1"
  local expect_presence="$2"
  local log
  log="$(mktemp)"
  set +e
  RUST_LOG=info ROOM_ID="$room" timeout 8 cargo run -q -p host-server >"$log" 2>&1
  set -e

  if grep -qE "Unable to connect|zenoh open failed|Connection refused" "$log"; then
    echo "smoke-presence-slots: ok (units only; zenohd not reachable — skip host run)"
    rm -f "$log"
    return 2
  fi

  local ok=1
  if ! grep -qE "host-server x0022 starting" "$log"; then
    echo "smoke-presence-slots: missing host-server x0022 start banner (ROOM_ID=$room)"
    ok=0
  fi
  if ! grep -q "user_roots=3..6" "$log"; then
    echo "smoke-presence-slots: missing user_roots=3..6 in banner"
    ok=0
  fi
  if ! grep -q "presence_ttl_secs=3" "$log"; then
    echo "smoke-presence-slots: missing presence_ttl_secs=3 in banner"
    ok=0
  fi
  if ! grep -q "presence=${expect_presence}" "$log"; then
    echo "smoke-presence-slots: missing presence=${expect_presence} in banner"
    ok=0
  fi
  if ! grep -q "protocol_pin=alchemy-protocol/0.9.0-x0022" "$log"; then
    echo "smoke-presence-slots: missing PROTOCOL_PIN 0.9.0-x0022"
    ok=0
  fi
  if ! grep -q "published PresenceState revision=0" "$log"; then
    echo "smoke-presence-slots: missing initial PresenceState publish"
    ok=0
  fi
  if ! grep -q "published SceneManifest revision=" "$log"; then
    echo "smoke-presence-slots: missing SceneManifest publish"
    ok=0
  fi
  if ! grep -q "physics: steps=" "$log"; then
    echo "smoke-presence-slots: missing physics stats"
    ok=0
  fi

  if [[ "$ok" -ne 1 ]]; then
    echo "smoke-presence-slots: FAILED (ROOM_ID=$room)"
    cat "$log"
    rm -f "$log"
    return 1
  fi
  rm -f "$log"
  return 0
}

check_host_banner "local" "alchemy/dev/room/local/presence"
rc=$?
if [[ "$rc" -eq 2 ]]; then
  exit 0
fi
if [[ "$rc" -ne 0 ]]; then
  exit 1
fi

# Distinct ROOM_ID must appear in Zenoh key (isolation contract).
check_host_banner "alpha" "alchemy/dev/room/alpha/presence"
rc=$?
if [[ "$rc" -eq 2 ]]; then
  exit 0
fi
if [[ "$rc" -ne 0 ]]; then
  exit 1
fi

echo "smoke-presence-slots: ok (local + alpha keys; Leave/liveliness = instant reclaim, TTL fallback — see x0019)"
