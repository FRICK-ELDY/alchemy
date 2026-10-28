#!/usr/bin/env bash
# Build and pack Windows x64 tester ZIP (launcher + host-server + client).
#
# Guarantees: release .exe under dist/alchemy-windows-x64-<version>.zip with
# Start.cmd, README-testers.txt, build-info.json.
# Non-guarantees: auth/world/assets/zenohd; signing; auto-update; WSL Linux builds;
# assets-offline Coin fixture (compile-time path; not in ZIP).
#
# Must run on native Windows (Git Bash / windows-latest). WSL builds Linux bins.
#
# Progress → stderr. Machine-readable lines (PACKAGE_*=) → stdout only.
#
# Usage:
#   make pack-windows
#   VERSION=v0.0.0-dev.202610281915 make pack-windows
#   SKIP_BUILD=1 make pack-windows
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT"

log() { printf '%s\n' "$*" >&2; }

case "$(uname -s)" in
  MINGW*|MSYS*|CYGWIN*) ;;
  *)
    log "error: pack-windows must run on native Windows (Git Bash) or windows-latest CI"
    log "  (WSL/macOS/Linux cargo would not produce .exe)"
    log "  From WSL: push a v0.0.0-dev.* tag and let windows-release.yml pack."
    exit 1
    ;;
esac

VERSION="${VERSION:-v0.0.0-dev.$(TZ=Asia/Tokyo date +%Y%m%d%H%M)}"
if [[ ! "$VERSION" =~ ^v0\.0\.0-dev\.[0-9]{12}(\.[0-9]+)?$ ]]; then
  log "error: VERSION must look like v0.0.0-dev.YYYYMMDDHHMM (got: $VERSION)"
  exit 1
fi

OUT_DIR="${OUTPUT_DIRECTORY:-$ROOT/dist}"
mkdir -p "$OUT_DIR"
OUT_DIR="$(cd "$OUT_DIR" && pwd)"

PACKAGE_NAME="alchemy-windows-x64-${VERSION}"
STAGING_PARENT="$ROOT/.run/packages/$(python3 -c 'import uuid; print(uuid.uuid4().hex)')"
PACKAGE_ROOT="$STAGING_PARENT/$PACKAGE_NAME"
mkdir -p "$PACKAGE_ROOT"

cargo_target_dir() {
  local manifest="$1"
  cargo metadata --format-version 1 --no-deps --manifest-path "$manifest" \
    | python3 -c 'import json,sys; print(json.load(sys.stdin)["target_directory"])'
}

RUSTC_HOST="$(rustc -vV | sed -n 's/^host: //p')"
if [[ -z "$RUSTC_HOST" ]]; then
  log "error: could not read rustc host from rustc -vV"
  exit 1
fi

if [[ "${SKIP_BUILD:-}" != "1" ]]; then
  log "Building alchemy-launcher (release)..."
  cargo build --release --manifest-path "$ROOT/launcher/Cargo.toml"
  log "Building host-server + client (release)..."
  cargo build --release -p host-server -p client --manifest-path "$ROOT/client/Cargo.toml"
fi

LAUNCHER_TARGET="$(cargo_target_dir "$ROOT/launcher/Cargo.toml")"
CLIENT_TARGET="$(cargo_target_dir "$ROOT/client/Cargo.toml")"
log "launcher target_directory=$LAUNCHER_TARGET"
log "client target_directory=$CLIENT_TARGET"
log "rustc host=$RUSTC_HOST"

copy_bin() {
  local src="$1" dst="$2"
  if [[ ! -f "$src" ]]; then
    log "error: missing binary: $src (build first or unset SKIP_BUILD)"
    exit 1
  fi
  cp -f "$src" "$PACKAGE_ROOT/$dst"
}

copy_bin "$LAUNCHER_TARGET/release/alchemy-launcher.exe" alchemy-launcher.exe
copy_bin "$CLIENT_TARGET/release/host-server.exe" host-server.exe
copy_bin "$CLIENT_TARGET/release/client.exe" client.exe

cp -f "$ROOT/.scripts/distribution/Start.cmd" "$PACKAGE_ROOT/"
cp -f "$ROOT/.scripts/distribution/README-testers.txt" "$PACKAGE_ROOT/"
cp -f "$ROOT/.scripts/distribution/env.testers.example.cmd" "$PACKAGE_ROOT/"

GIT_SHA="$(git -C "$ROOT" rev-parse HEAD)"
if [[ -z "$(git -C "$ROOT" status --porcelain --untracked-files=all)" ]]; then
  SOURCE_TREE_CLEAN=true
else
  SOURCE_TREE_CLEAN=false
fi
BUILT_AT="$(TZ=Asia/Tokyo date +%Y-%m-%dT%H:%M:%S%z)"

# Host triple from rustc; pack does not pass --target, so the build target equals host.
python3 - "$PACKAGE_ROOT/build-info.json" "$VERSION" "$GIT_SHA" "$SOURCE_TREE_CLEAN" "$BUILT_AT" "$RUSTC_HOST" <<'PY'
import json, sys
path, version, git_sha, clean, built_at, rustc_host = sys.argv[1:7]
info = {
    "version": version,
    "gitSha": git_sha,
    "sourceTreeClean": clean == "true",
    "builtAt": built_at,
    "rustcHost": rustc_host,
    "files": [
        "alchemy-launcher.exe",
        "host-server.exe",
        "client.exe",
        "Start.cmd",
        "README-testers.txt",
        "env.testers.example.cmd",
        "build-info.json",
    ],
}
with open(path, "w", encoding="utf-8") as f:
    json.dump(info, f, indent=4)
    f.write("\n")
PY

ZIP_PATH="$OUT_DIR/${PACKAGE_NAME}.zip"
rm -f "$ZIP_PATH"
python3 - "$STAGING_PARENT" "$PACKAGE_NAME" "$ZIP_PATH" <<'PY'
import sys, zipfile
from pathlib import Path
staging, name, zip_path = sys.argv[1:4]
root = Path(staging) / name
with zipfile.ZipFile(zip_path, "w", compression=zipfile.ZIP_DEFLATED) as zf:
    for path in sorted(root.rglob("*")):
        if path.is_file():
            zf.write(path, path.relative_to(root.parent).as_posix())
PY

EXTRACTED="$OUT_DIR/$PACKAGE_NAME"
rm -rf "$EXTRACTED"
cp -a "$PACKAGE_ROOT" "$EXTRACTED"
rm -rf "$STAGING_PARENT"

log "packed $ZIP_PATH"
printf '%s\n' \
  "PACKAGE_VERSION=$VERSION" \
  "PACKAGE_ROOT=$EXTRACTED" \
  "PACKAGE_ZIP=$ZIP_PATH" \
  "GIT_SHA=$GIT_SHA"
