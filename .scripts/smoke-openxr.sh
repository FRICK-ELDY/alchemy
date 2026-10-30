#!/usr/bin/env bash
# x000b–x0010: OpenXR render models + laser + grabbable box hit + loco tests + compile.
set -euo pipefail
ROOT="$(cd "$(dirname "$0")/.." && pwd)"
cd "$ROOT/client"

echo "== cargo test -p client (surface/xr + mesh flatten/textures/rig + shaders) =="
ALCHEMY_FORCE_DESKTOP=1 cargo test -p client -- --nocapture

echo "== cargo check -p client =="
ALCHEMY_FORCE_DESKTOP=1 cargo check -p client

echo "OK: OpenXR compile/unit gate passed (HMD not required; x000e–x0011 / x0050–x0052)."
echo "Note: this script uses ALCHEMY_FORCE_DESKTOP=1 — it does not present to an HMD."
echo "Manual HMD (Windows + SteamVR 2.12+): cargo run -p client  # omit FORCE_DESKTOP"
echo "  Controllers: XR_EXT_render_model GLB; baseColor + animatable nodes (trigger/stick)"
echo "  Menu toggles dash + cyan aim laser (hover lightens buttons); Select clicks Exit"
echo "  Dash closed: world laser hits grabbable boxes (highlight); Grab hold = x0011"
echo "  sticks Y=move X=turn; SceneGpu solid floor (x0052; not ash checker)"
echo "Force Desktop only: ALCHEMY_FORCE_DESKTOP=1 cargo run -p client"
