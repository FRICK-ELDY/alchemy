#!/usr/bin/env bash
# x001f: Hermetic Compose E2E — login → Room Token → input → RenderFrame.
# Requires: docker compose stack from docker-compose.e2e.yml already up (or COMPOSE_UP=1).
# Strict: missing services / timeout → exit 1 (never "ok (units only)").
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT"

# Defaults match docker-compose.e2e.yml host port offsets (coexist with :4002 stack).
AUTH_URL="${AUTH_URL:-http://127.0.0.1:4102}"
WORLD_URL="${WORLD_URL:-http://127.0.0.1:4103}"
ASSETS_URL="${ASSETS_URL:-http://127.0.0.1:4104}"
ZENOH_CONNECT="${ZENOH_CONNECT:-tcp/127.0.0.1:7547}"
ROOM_ID="${ROOM_ID:-local}"
AUTH_USER="${AUTH_USER:-alice}"
AUTH_PASSWORD="${AUTH_PASSWORD:-Password1}"
E2E_TIMEOUT_SECS="${E2E_TIMEOUT_SECS:-45}"
HOST_READY_SECS="${HOST_READY_SECS:-60}"
# Align with CI (cold mix compile can exceed 3m on first world/assets boot).
HEALTH_WAIT_SECS="${HEALTH_WAIT_SECS:-300}"
COMPOSE_FILE="${COMPOSE_FILE:-docker-compose.e2e.yml}"

HOST_LOG="${TMPDIR:-/tmp}/alchemy-e2e-host.log"
HOST_READY_FILE="${ALCHEMY_HOST_READY_FILE:-${TMPDIR:-/tmp}/alchemy-e2e-host.ready}"
HOST_PID=""

cleanup() {
  if [[ -n "${HOST_PID}" ]] && kill -0 "${HOST_PID}" 2>/dev/null; then
    kill "${HOST_PID}" 2>/dev/null || true
    wait "${HOST_PID}" 2>/dev/null || true
  fi
  rm -f "${HOST_READY_FILE}" 2>/dev/null || true
}
trap cleanup EXIT

wait_http() {
  local url="$1"
  local name="$2"
  local deadline=$((SECONDS + HEALTH_WAIT_SECS))
  echo "smoke-compose-e2e: waiting for ${name} at ${url} …"
  while (( SECONDS < deadline )); do
    if curl -sf --max-time 3 "${url}" >/dev/null 2>&1; then
      echo "smoke-compose-e2e: ${name} ok"
      return 0
    fi
    sleep 2
  done
  echo "smoke-compose-e2e: FAILED — ${name} not healthy within ${HEALTH_WAIT_SECS}s (${url})" >&2
  return 1
}

wait_tcp() {
  # ZENOH_CONNECT like tcp/127.0.0.1:7447
  local endpoint="$1"
  local host port
  host="$(printf '%s' "${endpoint}" | sed -n 's|^tcp/\([^:]*\):\([0-9]*\)$|\1|p')"
  port="$(printf '%s' "${endpoint}" | sed -n 's|^tcp/\([^:]*\):\([0-9]*\)$|\2|p')"
  if [[ -z "${host}" || -z "${port}" ]]; then
    echo "smoke-compose-e2e: FAILED — bad ZENOH_CONNECT=${endpoint}" >&2
    return 1
  fi
  local deadline=$((SECONDS + HEALTH_WAIT_SECS))
  echo "smoke-compose-e2e: waiting for zenohd ${host}:${port} …"
  while (( SECONDS < deadline )); do
    if (echo >/dev/tcp/"${host}"/"${port}") >/dev/null 2>&1; then
      echo "smoke-compose-e2e: zenohd ok"
      return 0
    fi
    if command -v nc >/dev/null 2>&1 && nc -z "${host}" "${port}" >/dev/null 2>&1; then
      echo "smoke-compose-e2e: zenohd ok"
      return 0
    fi
    sleep 1
  done
  echo "smoke-compose-e2e: FAILED — zenohd not accepting ${endpoint}" >&2
  return 1
}

if [[ "${COMPOSE_UP:-0}" == "1" ]]; then
  echo "smoke-compose-e2e: COMPOSE_UP=1 → docker compose -f ${COMPOSE_FILE} up -d --build"
  docker compose -f "${COMPOSE_FILE}" up -d --build
fi

if ! command -v curl >/dev/null 2>&1; then
  echo "smoke-compose-e2e: FAILED — curl required" >&2
  exit 1
fi

wait_http "${AUTH_URL}/health" "auth"
wait_http "${WORLD_URL}/health" "world"
wait_http "${ASSETS_URL}/health" "assets"
wait_tcp "${ZENOH_CONNECT}"

echo "smoke-compose-e2e: building host-server + e2e-probe …"
(
  cd "${ROOT}/client"
  cargo build -p host-server -p e2e-probe
)

rm -f "${HOST_READY_FILE}"
: >"${HOST_LOG}"
echo "smoke-compose-e2e: starting host-server (log ${HOST_LOG}, ready ${HOST_READY_FILE}) …"
# Use `cargo run` so CARGO_TARGET_DIR / custom target dirs are respected (not hard-coded paths).
export AUTH_URL WORLD_URL ZENOH_CONNECT ROOM_ID AUTH_USER AUTH_PASSWORD
export ALCHEMY_HOST_READY_FILE="${HOST_READY_FILE}"
(
  cd "${ROOT}/client"
  RUST_LOG="${RUST_LOG:-info}" cargo run -q -p host-server
) >"${HOST_LOG}" 2>&1 &
HOST_PID=$!

host_deadline=$((SECONDS + HOST_READY_SECS))
while (( SECONDS < host_deadline )); do
  if ! kill -0 "${HOST_PID}" 2>/dev/null; then
    echo "smoke-compose-e2e: FAILED — host-server exited early" >&2
    cat "${HOST_LOG}" >&2 || true
    exit 1
  fi
  if [[ -f "${HOST_READY_FILE}" ]]; then
    echo "smoke-compose-e2e: host-server ready (ready file)"
    break
  fi
  sleep 1
done
if [[ ! -f "${HOST_READY_FILE}" ]]; then
  echo "smoke-compose-e2e: FAILED — host-server ready file not written within ${HOST_READY_SECS}s (${HOST_READY_FILE})" >&2
  cat "${HOST_LOG}" >&2 || true
  exit 1
fi

echo "smoke-compose-e2e: running e2e-probe …"
export ASSETS_URL E2E_TIMEOUT_SECS
if ! (
  cd "${ROOT}/client"
  cargo run -q -p e2e-probe
); then
  echo "smoke-compose-e2e: FAILED — e2e-probe" >&2
  echo "---- host-server log ----" >&2
  cat "${HOST_LOG}" >&2 || true
  exit 1
fi

echo "smoke-compose-e2e: ok (login→token→input→frame)"
