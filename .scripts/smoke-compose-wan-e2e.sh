#!/usr/bin/env bash
# x0070: WAN negative E2E — mTLS zenohd + wildcard other-room ciphertext unreadable + no-cert reject.
# Requires: docker, openssl, curl. Generates ephemeral certs (not committed).
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT"

AUTH_URL="${AUTH_URL:-http://127.0.0.1:4102}"
WORLD_URL="${WORLD_URL:-http://127.0.0.1:4103}"
ASSETS_URL="${ASSETS_URL:-http://127.0.0.1:4104}"
ZENOH_CONNECT="${ZENOH_CONNECT:-tls/127.0.0.1:7547}"
ROOM_ID="${ROOM_ID:-wana}"
ROOM_ID_B="${ROOM_ID_B:-wanb}"
AUTH_USER="${AUTH_USER:-alice}"
AUTH_PASSWORD="${AUTH_PASSWORD:-Password1}"
E2E_TIMEOUT_SECS="${E2E_TIMEOUT_SECS:-60}"
HEALTH_WAIT_SECS="${HEALTH_WAIT_SECS:-300}"
COMPOSE_FILE="${COMPOSE_FILE:-docker-compose.e2e.yml}"
COMPOSE_WAN="${COMPOSE_WAN:-docker-compose.e2e.wan.yml}"

E2E_TLS_DIR="${E2E_TLS_DIR:-${TMPDIR:-/tmp}/alchemy-e2e-wan-tls}"
export E2E_TLS_DIR

cleanup() {
  if [[ "${COMPOSE_DOWN:-1}" == "1" ]] && [[ "${KEEP_COMPOSE:-0}" != "1" ]]; then
    docker compose -f "${COMPOSE_FILE}" -f "${COMPOSE_WAN}" down -v >/dev/null 2>&1 || true
  fi
}
trap cleanup EXIT

wait_http() {
  local url="$1"
  local name="$2"
  local deadline=$((SECONDS + HEALTH_WAIT_SECS))
  echo "smoke-compose-wan-e2e: waiting for ${name} at ${url} …"
  while (( SECONDS < deadline )); do
    if curl -sf --max-time 3 "${url}" >/dev/null 2>&1; then
      echo "smoke-compose-wan-e2e: ${name} ok"
      return 0
    fi
    sleep 2
  done
  echo "smoke-compose-wan-e2e: FAILED — ${name} not healthy within ${HEALTH_WAIT_SECS}s (${url})" >&2
  return 1
}

wait_tcp() {
  local host="$1"
  local port="$2"
  local deadline=$((SECONDS + HEALTH_WAIT_SECS))
  echo "smoke-compose-wan-e2e: waiting for tls ${host}:${port} …"
  while (( SECONDS < deadline )); do
    if (echo >/dev/tcp/"${host}"/"${port}") >/dev/null 2>&1; then
      echo "smoke-compose-wan-e2e: zenohd tcp accept ok"
      return 0
    fi
    if command -v nc >/dev/null 2>&1 && nc -z "${host}" "${port}" >/dev/null 2>&1; then
      echo "smoke-compose-wan-e2e: zenohd tcp accept ok"
      return 0
    fi
    sleep 1
  done
  echo "smoke-compose-wan-e2e: FAILED — zenohd not accepting ${host}:${port}" >&2
  return 1
}

if ! command -v curl >/dev/null 2>&1; then
  echo "smoke-compose-wan-e2e: FAILED — curl required" >&2
  exit 1
fi
if ! command -v openssl >/dev/null 2>&1; then
  echo "smoke-compose-wan-e2e: FAILED — openssl required" >&2
  exit 1
fi

rm -rf "${E2E_TLS_DIR}"
mkdir -p "${E2E_TLS_DIR}"
echo "smoke-compose-wan-e2e: generating WAN certs in ${E2E_TLS_DIR} …"
bash "${ROOT}/relay-server/scripts/gen-wan-dev-certs.sh" "127.0.0.1" "${E2E_TLS_DIR}"

echo "smoke-compose-wan-e2e: compose up (hermetic + WAN overlay) …"
docker compose -f "${COMPOSE_FILE}" -f "${COMPOSE_WAN}" up -d --build

wait_http "${AUTH_URL}/health" "auth"
wait_http "${WORLD_URL}/health" "world"
wait_http "${ASSETS_URL}/health" "assets"
wait_tcp "127.0.0.1" "7547"

export AUTH_URL WORLD_URL ASSETS_URL ZENOH_CONNECT ROOM_ID ROOM_ID_B AUTH_USER AUTH_PASSWORD
export E2E_TIMEOUT_SECS TRUST_PROBE_MODE=wan
export ZENOH_TLS_ROOT_CA="${E2E_TLS_DIR}/ca-cert.pem"

echo "smoke-compose-wan-e2e: building + running trust-probe WAN …"
if ! (
  cd "${ROOT}/client"
  cargo run -q -p alchemy-client-trust --bin trust-probe
); then
  echo "smoke-compose-wan-e2e: FAILED — trust-probe WAN" >&2
  docker compose -f "${COMPOSE_FILE}" -f "${COMPOSE_WAN}" logs --no-color --tail=120 >&2 || true
  exit 1
fi

echo "smoke-compose-wan-e2e: ok (no-cert reject + wildcard other-room unreadable)"
