#!/bin/sh
# x001e: named volumes often mount as root; repair ownership then drop to `assets`.
# Prefer migrate via compose profile `assets-migrate` (always goes through this entrypoint).
set -eu
mkdir -p /data
# Fix any node under /data not owned by assets (covers root-owned SQLite after a bad `exec`).
# No-op when clean: find still walks, but avoids blanket chown -R every boot.
if find /data ! -user assets -print -quit 2>/dev/null | grep -q .; then
  find /data ! -user assets -exec chown assets:assets {} +
fi
exec su-exec assets "$@"
