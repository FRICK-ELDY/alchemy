#!/usr/bin/env bash
# x004c / P3-9 — short cargo-fuzz smoke (Linux / macOS; not Windows MSVC).
# Usage: ./.scripts/ci-fuzz.sh [protocol|host|all]
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
cd "$ROOT"

MAX_TIME="${FUZZ_MAX_TOTAL_TIME:-8}"
# Default stays small for CI throughput. Payload byte-gate oversize paths are covered by
# protocol unit tests (`decode_*_checked` / `check_payload_bytes`), not this smoke.
MAX_LEN="${FUZZ_MAX_LEN:-8192}"

if [[ "$(uname -s)" == MINGW* || "$(uname -s)" == CYGWIN* || "$(uname -s)" == MSYS* ]]; then
  echo "[SKIP] cargo-fuzz requires Unix + nightly libFuzzer (use Linux CI / WSL)" >&2
  exit 0
fi

ensure_fuzz() {
  if ! command -v cargo-fuzz >/dev/null 2>&1 && ! cargo fuzz --help >/dev/null 2>&1; then
    echo "== install cargo-fuzz =="
    cargo install cargo-fuzz --locked
  fi
  rustup toolchain install nightly --profile minimal -c rust-src >/dev/null
}

run_target() {
  local dir="$1"
  local target="$2"
  echo "== fuzz ${dir} :: ${target} (${MAX_TIME}s) =="
  (
    cd "$dir"
    cargo +nightly fuzz run "$target" -- \
      -max_total_time="${MAX_TIME}" \
      -max_len="${MAX_LEN}"
  )
}

run_protocol() {
  ensure_fuzz
  for t in decode_manifest decode_frame decode_presence decode_input; do
    run_target protocol "$t"
  done
}

run_host() {
  ensure_fuzz
  run_target client/host-server room_token_verify
}

case "${1:-all}" in
  protocol) run_protocol ;;
  host) run_host ;;
  all)
    run_protocol
    run_host
    ;;
  *)
    echo "usage: $0 [protocol|host|all]" >&2
    exit 2
    ;;
esac

echo "ci-fuzz OK"
