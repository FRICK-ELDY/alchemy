#!/bin/sh
# x001e: named volumes often mount as root; repair ownership then drop to `world`.
set -eu
mkdir -p /var/lib/world
if find /var/lib/world ! -user world -print -quit 2>/dev/null | grep -q .; then
  find /var/lib/world ! -user world -exec chown world:world {} +
fi
exec su-exec world "$@"
