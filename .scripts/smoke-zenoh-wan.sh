#!/usr/bin/env bash
# x0022 / x001b: Zenoh WAN — TCP reachability gate for ZENOH_CONNECT (tls/ or legacy tcp/).
# Full 2-client Presence / Leave is an operator demo (see manual).
set -euo pipefail

usage() {
  cat <<'EOF'
usage: ZENOH_CONNECT=tls/<VPS_HOST>:7447 bash .scripts/smoke-zenoh-wan.sh
   or: ZENOH_CONNECT=tcp/<VPS_HOST>:7447 bash .scripts/smoke-zenoh-wan.sh  # legacy

Checks that the host:port in ZENOH_CONNECT accepts TCP connections (allowlist).
Does not verify the TLS handshake. Operator demo:
  .workspace/0_doc/manual/zenoh-wan-conoha.md
EOF
}

if [[ -z "${ZENOH_CONNECT:-}" ]]; then
  usage
  exit 1
fi

endpoint="$ZENOH_CONNECT"
case "$endpoint" in
  tls/*) endpoint="${endpoint#tls/}" ;;
  tcp/*) endpoint="${endpoint#tcp/}" ;;
  *)
    echo "smoke-zenoh-wan: ZENOH_CONNECT must look like tls/<host>:7447 (got: ${ZENOH_CONNECT})"
    exit 1
    ;;
esac

host="${endpoint%:*}"
port="${endpoint##*:}"
host="${host#[}"
host="${host%]}"

if [[ -z "$host" || -z "$port" || "$host" == "$port" ]]; then
  echo "smoke-zenoh-wan: could not parse host:port from ${ZENOH_CONNECT}"
  exit 1
fi

echo "smoke-zenoh-wan: probing ${host}:${port} …"
if command -v nc >/dev/null 2>&1; then
  if nc -vz -w 5 "$host" "$port"; then
    echo "smoke-zenoh-wan: ok (TCP accept; TLS handshake not checked)"
    exit 0
  fi
  echo "smoke-zenoh-wan: FAILED (nc)"
  exit 1
fi

# Fallback: bash /dev/tcp with an outer timeout so filter drops cannot hang forever.
if command -v timeout >/dev/null 2>&1; then
  if timeout 5 bash -c "echo >/dev/tcp/${host}/${port}" >/dev/null 2>&1; then
    echo "smoke-zenoh-wan: ok (TCP accept via /dev/tcp; TLS handshake not checked)"
    exit 0
  fi
  echo "smoke-zenoh-wan: FAILED (/dev/tcp or timeout)"
  exit 1
fi

echo "smoke-zenoh-wan: FAILED (need nc or GNU timeout + bash /dev/tcp)"
exit 1
