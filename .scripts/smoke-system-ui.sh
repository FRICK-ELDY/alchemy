#!/usr/bin/env bash
# Slice E: compile-gate for system_ui + host_control wiring (no GUI assertion).
set -euo pipefail
ROOT="$(cd "$(dirname "$0")/.." && pwd)"
cd "$ROOT/client"

echo "== cargo test -p client (system_ui unit) =="
cargo test -p client -- --nocapture

echo "== cargo check -p host-server =="
cargo check -p host-server

echo "== protocol host_control key =="
cd "$ROOT/protocol"
cargo test key_helpers -- --nocapture

echo "OK: system_ui / Dash Facet (x000f) compile/smoke gate passed."
echo "Manual: Esc opens dash; hover lightens buttons; Exit client only / Exit client + host (ALCHEMY_HOST_PID or host_control)."
