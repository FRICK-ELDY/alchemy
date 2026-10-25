#!/usr/bin/env bash
# x000b–x000e: OpenXR render models + laser + loco tests + compile.
set -euo pipefail
ROOT="$(cd "$(dirname "$0")/.." && pwd)"
cd "$ROOT/client"

echo "== cargo test -p client (surface/xr + mesh flatten/textures/rig + shaders) =="
ALCHEMY_FORCE_DESKTOP=1 cargo test -p client -- --nocapture

echo "== cargo check -p client =="
ALCHEMY_FORCE_DESKTOP=1 cargo check -p client

echo "OK: OpenXR compile/smoke gate passed (HMD not required; x000e/x000f)."
echo "Manual (Windows + SteamVR 2.12+): cargo run -p client"
echo "  Controllers: XR_EXT_render_model GLB; baseColor + animatable nodes (trigger/stick)"
echo "  Menu toggles dash + cyan aim laser (hover lightens buttons); Select clicks Exit"
echo "  sticks Y=move X=turn; floor"
echo "Force Desktop: ALCHEMY_FORCE_DESKTOP=1 cargo run -p client"
