#!/usr/bin/env bash
# x0009: host publishes Manifest with Noclip + user_root; Rapier still ticks.
# Requires: auth-server, world-server, zenohd on :7447.
set -euo pipefail
ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT/client"

cargo build -q -p host-server
cargo test -q -p host-server -- sim::tests::noclip_passes_through_floor --exact

LOG="$(mktemp)"
set +e
RUST_LOG=info timeout 8 cargo run -q -p host-server >"$LOG" 2>&1
set -e

ok=1
if ! grep -q "host-server x0009 starting" "$LOG"; then
  echo "smoke-desktop-noclip: missing x0009 banner"
  ok=0
fi
if ! grep -q "protocol_pin=alchemy-protocol/0.4.0-x0009" "$LOG"; then
  echo "smoke-desktop-noclip: missing PROTOCOL_PIN"
  ok=0
fi
if ! grep -q "loco=Noclip" "$LOG"; then
  echo "smoke-desktop-noclip: missing Noclip locomotion in Manifest log"
  ok=0
fi
if ! grep -q "user_root" "$LOG"; then
  echo "smoke-desktop-noclip: missing user_root in ready/stats log"
  ok=0
fi
if ! grep -q "published SceneManifest revision=" "$LOG"; then
  echo "smoke-desktop-noclip: missing SceneManifest publish"
  ok=0
fi
if ! grep -q "physics: steps=" "$LOG"; then
  echo "smoke-desktop-noclip: missing physics stats"
  ok=0
fi

if [[ "$ok" -ne 1 ]]; then
  echo "smoke-desktop-noclip: FAILED"
  cat "$LOG"
  rm -f "$LOG"
  exit 1
fi

rm -f "$LOG"
echo "smoke-desktop-noclip: ok"
