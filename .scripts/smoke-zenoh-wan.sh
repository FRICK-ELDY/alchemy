#!/usr/bin/env bash
# x001b: Zenoh WAN (ConoHa) — connectivity gate for ZENOH_CONNECT.
# Full 2-client Presence / Leave is an operator demo (see manual).
set -euo pipefail

usage() {
  cat <<'EOF'
usage: ZENOH_CONNECT=tcp/<VPS_IP>:7447 bash .scripts/smoke-zenoh-wan.sh

Checks that the TCP endpoint in ZENOH_CONNECT accepts connections (allowlist).
Does not run host/client. Operator demo:
  .workspace/0_doc/manual/zenoh-wan-conoha.md
EOF
}

if [[ -z "${ZENOH_CONNECT:-}" ]]; then
  usage
  exit 1
fi

# Expect forms like tcp/1.2.3.4:7447 or tcp/[::1]:7447
endpoint="${ZENOH_CONNECT#tcp/}"
if [[ "$endpoint" == "$ZENOH_CONNECT" ]]; then
  echo "smoke-zenoh-wan: ZENOH_CONNECT must look like tcp/<host>:7447 (got: ${ZENOH_CONNECT})"
  exit 1
fi

host="${endpoint%:*}"
port="${endpoint##*:}"
# strip brackets for IPv6 literals if present
host="${host#[}"
host="${host%]}"

if [[ -z "$host" || -z "$port" || "$host" == "$port" ]]; then
  echo "smoke-zenoh-wan: could not parse host:port from ${ZENOH_CONNECT}"
  exit 1
fi

echo "smoke-zenoh-wan: probing ${host}:${port} …"
if command -v nc >/dev/null 2>&1; then
  if nc -z -w 5 "$host" "$port" 2>/dev/null; then
    echo "smoke-zenoh-wan: ok (TCP accept on ${ZENOH_CONNECT})"
    echo "smoke-zenoh-wan: next — 2 clients Presence/Leave per manual/zenoh-wan-conoha.md"
    exit 0
  fi
  echo "smoke-zenoh-wan: TCP probe failed (ufw allowlist / zenohd down / wrong IP?)"
  exit 1
fi

# Fallback: bash /dev/tcp (bash only)
if timeout 5 bash -c "echo >/dev/tcp/${host}/${port}" 2>/dev/null; then
  echo "smoke-zenoh-wan: ok (TCP accept on ${ZENOH_CONNECT})"
  echo "smoke-zenoh-wan: next — 2 clients Presence/Leave per manual/zenoh-wan-conoha.md"
  exit 0
fi

echo "smoke-zenoh-wan: TCP probe failed (install nc, or check allowlist / zenohd)"
exit 1
