#!/usr/bin/env bash
# x0070 / x0087: WAN E2E — mTLS zenohd + real host-server round-trip + negatives.
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
E2E_TIMEOUT_SECS="${E2E_TIMEOUT_SECS:-90}"
HOST_READY_SECS="${HOST_READY_SECS:-120}"
HEALTH_WAIT_SECS="${HEALTH_WAIT_SECS:-300}"
COMPOSE_FILE="${COMPOSE_FILE:-docker-compose.e2e.yml}"
COMPOSE_WAN="${COMPOSE_WAN:-docker-compose.e2e.wan.yml}"

E2E_TLS_DIR="${E2E_TLS_DIR:-${TMPDIR:-/tmp}/alchemy-e2e-wan-tls}"
export E2E_TLS_DIR

HOST_LOG="${TMPDIR:-/tmp}/alchemy-e2e-wan-host.log"
HOST_READY_FILE="${ALCHEMY_HOST_READY_FILE:-${TMPDIR:-/tmp}/alchemy-e2e-wan-host.ready}"
HOST_EXIT_FILE="${TMPDIR:-/tmp}/alchemy-e2e-wan-host.exit"
PROBE_EXIT_FILE="${TMPDIR:-/tmp}/alchemy-e2e-wan-probe.exit"
HOST_PID=""
PROBE_PID=""
HOST_WATCH_PID=""

# Prefer setsid (Linux CI). Git Bash/MSYS often lacks it — fall back to job-control PG.
HAVE_SETSID=0
if command -v setsid >/dev/null 2>&1; then
  HAVE_SETSID=1
fi

# Signal a process-group leader (and children). Caller waits if it owns the PID.
kill_pg() {
  local pid="$1"
  [[ -n "${pid}" ]] || return 0
  kill -- -"${pid}" 2>/dev/null || kill "${pid}" 2>/dev/null || true
}

# Kill a PID and its process group when possible.
kill_tree() {
  local pid="$1"
  kill_pg "${pid}"
  wait "${pid}" 2>/dev/null || true
}

# Run a command string in its own process group; echo PID on stdout via caller capture.
# Usage: run_pg_cmd LOGFILE 'shell commands…'  → sets RUN_PG_PID
run_pg_cmd() {
  local logfile="${1:-}"
  local cmd="$2"
  if [[ "${HAVE_SETSID}" -eq 1 ]]; then
    if [[ -n "${logfile}" ]]; then
      setsid bash -c "${cmd}" >"${logfile}" 2>&1 &
    else
      setsid bash -c "${cmd}" &
    fi
    RUN_PG_PID=$!
    return 0
  fi
  # No setsid: enable monitor mode so background jobs get their own PGID (== PID).
  set -m
  if [[ -n "${logfile}" ]]; then
    bash -c "${cmd}" >"${logfile}" 2>&1 &
  else
    bash -c "${cmd}" &
  fi
  RUN_PG_PID=$!
}

cleanup() {
  if [[ -n "${HOST_WATCH_PID}" ]]; then
    kill "${HOST_WATCH_PID}" 2>/dev/null || true
    wait "${HOST_WATCH_PID}" 2>/dev/null || true
    HOST_WATCH_PID=""
  fi
  if [[ -n "${PROBE_PID}" ]]; then
    kill_tree "${PROBE_PID}"
    PROBE_PID=""
  fi
  if [[ -n "${HOST_PID}" ]]; then
    kill_tree "${HOST_PID}"
    HOST_PID=""
  fi
  rm -f "${HOST_READY_FILE}" "${HOST_EXIT_FILE}" "${PROBE_EXIT_FILE}" 2>/dev/null || true
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
if [[ "${HAVE_SETSID}" -eq 0 ]]; then
  echo "smoke-compose-wan-e2e: setsid not found — using bash job-control process groups (Git Bash/MSYS)"
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
export E2E_TIMEOUT_SECS ZENOH_TLS_ROOT_CA="${E2E_TLS_DIR}/ca-cert.pem"

echo "smoke-compose-wan-e2e: building host-server + trust-probe …"
(
  cd "${ROOT}/client"
  cargo build -p host-server -p alchemy-client-trust
)

rm -f "${HOST_READY_FILE}" "${HOST_EXIT_FILE}" "${PROBE_EXIT_FILE}"
: >"${HOST_LOG}"
echo "smoke-compose-wan-e2e: starting host-server (log ${HOST_LOG}, ready ${HOST_READY_FILE}) …"
# x0087: real host on WAN TLS — CSR via should_request_world_mtls (ZENOH_CONNECT=tls/…).
# Exit files (not kill -0): zombies still succeed kill -0 and would hang CI.
export ALCHEMY_HOST_READY_FILE="${HOST_READY_FILE}"
# Own process group so cleanup/watcher can kill cargo children without orphans.
run_pg_cmd "${HOST_LOG}" "
  cd \"${ROOT}/client\"
  set +e
  RUST_LOG=\"${RUST_LOG:-info}\" cargo run -q -p host-server
  ec=\$?
  set -e
  echo \"\${ec}\" >\"${HOST_EXIT_FILE}\"
"
HOST_PID="${RUN_PG_PID}"

host_deadline=$((SECONDS + HOST_READY_SECS))
while (( SECONDS < host_deadline )); do
  if [[ -f "${HOST_EXIT_FILE}" ]]; then
    echo "smoke-compose-wan-e2e: FAILED — host-server exited early (rc=$(cat "${HOST_EXIT_FILE}"))" >&2
    cat "${HOST_LOG}" >&2 || true
    exit 1
  fi
  if [[ -f "${HOST_READY_FILE}" ]]; then
    echo "smoke-compose-wan-e2e: host-server ready (ready file)"
    break
  fi
  sleep 1
done
if [[ ! -f "${HOST_READY_FILE}" ]]; then
  echo "smoke-compose-wan-e2e: FAILED — host-server ready file not written within ${HOST_READY_SECS}s (${HOST_READY_FILE})" >&2
  cat "${HOST_LOG}" >&2 || true
  exit 1
fi

echo "smoke-compose-wan-e2e: building + running trust-probe WAN (x0070+x0087) …"
export TRUST_PROBE_MODE=wan
run_pg_cmd "" "
  cd \"${ROOT}/client\"
  set +e
  cargo run -q -p alchemy-client-trust --bin trust-probe
  ec=\$?
  set -e
  echo \"\${ec}\" >\"${PROBE_EXIT_FILE}\"
"
PROBE_PID="${RUN_PG_PID}"

# Ready file is before first Manifest seal; watch host via exit file while probe runs.
# Failure signal is HOST_EXIT_FILE only — do not treat intentional watcher SIGTERM as host death.
(
  while [[ ! -f "${PROBE_EXIT_FILE}" ]]; do
    if [[ -f "${HOST_EXIT_FILE}" ]]; then
      echo "smoke-compose-wan-e2e: FAILED — host-server exited while trust-probe was running (rc=$(cat "${HOST_EXIT_FILE}"))" >&2
      kill_pg "${PROBE_PID}"
      exit 1
    fi
    sleep 1
  done
  exit 0
) &
HOST_WATCH_PID=$!

set +e
wait "${PROBE_PID}"
PROBE_WAIT_RC=$?
set -e
PROBE_PID=""

# Stop watcher; its exit code is irrelevant (may be 0 or SIGTERM).
kill "${HOST_WATCH_PID}" 2>/dev/null || true
wait "${HOST_WATCH_PID}" 2>/dev/null || true
HOST_WATCH_PID=""

if [[ -f "${HOST_EXIT_FILE}" ]]; then
  echo "smoke-compose-wan-e2e: FAILED — host-server exited while trust-probe was running (rc=$(cat "${HOST_EXIT_FILE}"))" >&2
  echo "---- host-server log ----" >&2
  cat "${HOST_LOG}" >&2 || true
  docker compose -f "${COMPOSE_FILE}" -f "${COMPOSE_WAN}" logs --no-color --tail=120 >&2 || true
  exit 1
fi

PROBE_RC="${PROBE_WAIT_RC}"
if [[ -f "${PROBE_EXIT_FILE}" ]]; then
  PROBE_RC="$(cat "${PROBE_EXIT_FILE}")"
fi
if [[ "${PROBE_RC}" -ne 0 ]]; then
  echo "smoke-compose-wan-e2e: FAILED — trust-probe WAN (rc=${PROBE_RC})" >&2
  echo "---- host-server log ----" >&2
  cat "${HOST_LOG}" >&2 || true
  docker compose -f "${COMPOSE_FILE}" -f "${COMPOSE_WAN}" logs --no-color --tail=120 >&2 || true
  exit 1
fi

echo "smoke-compose-wan-e2e: ok (host Accept/Frame + no-cert + guest put deny + old epoch + wildcard unreadable)"
