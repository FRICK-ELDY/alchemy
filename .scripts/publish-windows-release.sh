#!/usr/bin/env bash
# Tag v0.0.0-dev.* and push so CI (windows-release.yml) packs and publishes the Pre-release.
#
# Guarantees: annotated tag on HEAD (optional create/push). Does **not** call gh release
# (avoids racing CI on the same tag).
# Non-guarantees: local pack equals CI artifact; SmartScreen; server stack in the ZIP.
#
# Canonical publish path: CREATE_TAG=1 PUSH_TAG=1 → tag push → windows-release.yml.
# Optional local pack (native Windows only) is for smoke; Release assets always come from CI.
#
# Usage:
#   make publish-windows CREATE_TAG=1 PUSH_TAG=1
#   VERSION=v0.0.0-dev.202610281915 make publish-windows CREATE_TAG=1 PUSH_TAG=1
#   SKIP_PACK=1 make publish-windows CREATE_TAG=1 PUSH_TAG=1   # WSL / tag-only
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT"

log() { printf '%s\n' "$*" >&2; }

VERSION="${VERSION:-v0.0.0-dev.$(TZ=Asia/Tokyo date +%Y%m%d%H%M)}"
if [[ ! "$VERSION" =~ ^v0\.0\.0-dev\.[0-9]{12}(\.[0-9]+)?$ ]]; then
  log "error: VERSION must look like v0.0.0-dev.YYYYMMDDHHMM (got: $VERSION)"
  exit 1
fi

COMMIT="$(git rev-parse HEAD)"
if [[ -n "$(git status --porcelain --untracked-files=all)" ]]; then
  log "error: working tree is dirty; commit or stash before publishing"
  exit 1
fi

# Default: skip local pack (WSL-safe). Set SKIP_PACK=0 on native Windows to smoke-pack.
if [[ "${SKIP_PACK:-1}" != "1" ]]; then
  export VERSION
  log "Local smoke pack (Release upload is still CI-only)..."
  ./.scripts/pack-windows-zip.sh >/dev/null
fi

if git rev-parse -q --verify "refs/tags/$VERSION" >/dev/null; then
  TAG_COMMIT="$(git rev-list -n 1 "refs/tags/$VERSION")"
  if [[ "$TAG_COMMIT" != "$COMMIT" ]]; then
    log "error: tag $VERSION points to $TAG_COMMIT, not HEAD $COMMIT"
    exit 1
  fi
elif [[ "${CREATE_TAG:-}" == "1" ]]; then
  git tag -a "$VERSION" -m "Alchemy Windows tester $VERSION"
else
  log "error: tag $VERSION missing; pass CREATE_TAG=1 or create it first"
  exit 1
fi

if [[ "${PUSH_TAG:-}" == "1" ]]; then
  git push origin "refs/tags/$VERSION"
  log "Pushed $VERSION — windows-release.yml will pack and create the Pre-release."
  log "RELEASE_URL is valid after that workflow finishes."
  printf '%s\n' \
    "RELEASE_TAG=$VERSION" \
    "RELEASE_COMMIT=$COMMIT" \
    "RELEASE_URL=https://github.com/FRICK-ELDY/alchemy/releases/tag/$VERSION"
else
  log "Tag $VERSION is local only. Push with PUSH_TAG=1 (or git push --tags) for CI release."
  printf '%s\n' \
    "RELEASE_TAG=$VERSION" \
    "RELEASE_COMMIT=$COMMIT"
fi
