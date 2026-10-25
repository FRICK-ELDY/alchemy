#!/usr/bin/env bash
# x000b/x000c/x000d: OpenXR render models + baseColor + animatable nodes + loco tests + compile.
set -euo pipefail
ROOT="$(cd "$(dirname "$0")/.." && pwd)"
cd "$ROOT/client"

echo "== cargo test -p client (surface/xr + mesh flatten/textures/rig + shaders) =="
ALCHEMY_FORCE_DESKTOP=1 cargo test -p client -- --nocapture

echo "== cargo check -p client =="
ALCHEMY_FORCE_DESKTOP=1 cargo check -p client

echo "OK: x000d OpenXR compile/smoke gate passed (HMD not required)."
echo "Manual (Windows + SteamVR 2.12+): cargo run -p client"
echo "  Controllers: XR_EXT_render_model GLB; baseColor + animatable nodes (trigger/stick)"
echo "  Menu toggles dash; both sticks: Y=aim move, X=snap/smooth turn; checker floor"
echo "Force Desktop: ALCHEMY_FORCE_DESKTOP=1 cargo run -p client"
