#!/usr/bin/env bash
# Slice D: host runs Rapier 60Hz + publishes 30Hz deltas; Manifest still present.
# Requires: auth-server, world-server, zenohd on :7447.
set -euo pipefail
ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT/client"

cargo build -q -p host-server

LOG="$(mktemp)"
set +e
RUST_LOG=info timeout 8 cargo run -q -p host-server >"$LOG" 2>&1
set -e

ok=1
if ! grep -qE "host-server (Slice D|x0009) starting" "$LOG"; then
  echo "smoke-rapier-host: missing host-server start banner"
  ok=0
fi
if ! grep -q "physics_hz=60" "$LOG" || ! grep -q "net_hz=30" "$LOG"; then
  echo "smoke-rapier-host: missing dual-tick banner"
  ok=0
fi
if ! grep -q "Rapier world ready" "$LOG"; then
  echo "smoke-rapier-host: missing Rapier ready log"
  ok=0
fi
if ! grep -q "published SceneManifest revision=" "$LOG"; then
  echo "smoke-rapier-host: missing SceneManifest publish"
  ok=0
fi
if ! grep -q "physics: steps=" "$LOG"; then
  echo "smoke-rapier-host: missing physics stats"
  ok=0
fi

# Expect ~60 physics steps per second sample.
if ! python3 - "$LOG" <<'PY'
import re, sys
path = sys.argv[1]
pat = re.compile(r"physics: steps=(\d+).*delta: frames=(\d+)")
ok = False
for line in open(path, encoding="utf-8", errors="replace"):
    m = pat.search(line)
    if not m:
        continue
    steps, frames = int(m.group(1)), int(m.group(2))
    # Allow slack for timeout / scheduling.
    if 40 <= steps <= 80 and frames <= steps:
        ok = True
        print(f"dual-tick sample ok: physics_steps={steps} net_frames={frames}")
        break
sys.exit(0 if ok else 1)
PY
then
  echo "smoke-rapier-host: dual-tick rates look wrong"
  ok=0
fi

if [[ "$ok" -ne 1 ]]; then
  echo "smoke-rapier-host: FAILED"
  cat "$LOG"
  rm -f "$LOG"
  exit 1
fi

rm -f "$LOG"
echo "smoke-rapier-host: ok"
