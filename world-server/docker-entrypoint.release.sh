#!/bin/sh
# x001e: named volumes often mount as root; repair ownership then drop to `world`.
# x0028: migrate SQLite room ledger before boot.
set -eu
mkdir -p /var/lib/world
if find /var/lib/world ! -user world -print -quit 2>/dev/null | grep -q .; then
  find /var/lib/world ! -user world -exec chown world:world {} +
fi
su-exec world bin/world eval "World.Release.migrate()"
exec su-exec world "$@"
