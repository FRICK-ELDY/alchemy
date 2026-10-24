#!/usr/bin/env bash
# Slice C-C: host publishes SceneManifest once + delta frames (static entity omitted).
# Requires: auth-server, world-server, zenohd on :7447.
set -euo pipefail
ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT/client"

# Always rebuild so PROTOCOL_PIN / Slice C-C changes are not skipped by a stale binary.
cargo build -q -p host-server

LOG="$(mktemp)"
set +e
RUST_LOG=info timeout 6 cargo run -q -p host-server >"$LOG" 2>&1
set -e

ok=1
if ! grep -q "published SceneManifest revision=" "$LOG"; then
  echo "smoke-manifest-delta: missing SceneManifest publish"
  ok=0
fi
if ! grep -q "delta: frames=" "$LOG"; then
  echo "smoke-manifest-delta: missing delta stats"
  ok=0
fi

# After warm-up, moving entity alone ⇒ upsert_entities ≈ frames (not 2×).
# Accept any sample line where upsert_entities < 2 * frames (static slept).
if ! python3 - "$LOG" <<'PY'
import re, sys
path = sys.argv[1]
pat = re.compile(r"delta: frames=(\d+) upsert_entities=(\d+)")
ok = False
for line in open(path, encoding="utf-8", errors="replace"):
    m = pat.search(line)
    if not m:
        continue
    frames, upserts = int(m.group(1)), int(m.group(2))
    if frames >= 10 and upserts < frames * 2:
        ok = True
        print(f"delta sample ok: frames={frames} upsert_entities={upserts}")
        break
sys.exit(0 if ok else 1)
PY
then
  echo "smoke-manifest-delta: delta did not show static omission"
  ok=0
fi

if [[ "$ok" -ne 1 ]]; then
  echo "smoke-manifest-delta: FAILED"
  cat "$LOG"
  rm -f "$LOG"
  exit 1
fi

rm -f "$LOG"
echo "smoke-manifest-delta: ok"
