#!/usr/bin/env bash
# x0020: day-to-day stack smoke — host Token/Manifest/physics + client build/units.
# Requires: auth / world / assets / zenohd already up on defaults below.
# Stop any other host-server on the same ROOM_ID first (dual host / shared Zenoh keys).
# Strict: missing services / host failure → exit 1 (never "ok (units only)").
# Does NOT run Desktop client, e2e-probe, wgpu, or login→frame path (manual 手通しは README 3a).
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT"

# Canonical day-to-day ports (client / host-server defaults).
AUTH_URL="${AUTH_URL:-http://127.0.0.1:4002}"
WORLD_URL="${WORLD_URL:-http://127.0.0.1:4003}"
ASSETS_URL="${ASSETS_URL:-http://127.0.0.1:4004}"
ZENOH_CONNECT="${ZENOH_CONNECT:-tcp/127.0.0.1:7447}"
ROOM_ID="${ROOM_ID:-local}"
AUTH_USER="${AUTH_USER:-alice}"
AUTH_PASSWORD="${AUTH_PASSWORD:-Password1}"
HOST_READY_SECS="${HOST_READY_SECS:-60}"
HEALTH_WAIT_SECS="${HEALTH_WAIT_SECS:-30}"

HOST_LOG="${TMPDIR:-/tmp}/alchemy-desktop-handthrough-host.log"
HOST_READY_FILE="${ALCHEMY_HOST_READY_FILE:-${TMPDIR:-/tmp}/alchemy-desktop-handthrough-host.ready}"
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
  echo "smoke-desktop-handthrough: waiting for ${name} at ${url} …"
  while (( SECONDS < deadline )); do
    if curl -sf --max-time 3 "${url}" >/dev/null 2>&1; then
      echo "smoke-desktop-handthrough: ${name} ok"
      return 0
    fi
    sleep 1
  done
  echo "smoke-desktop-handthrough: FAILED — ${name} not healthy within ${HEALTH_WAIT_SECS}s (${url})" >&2
  echo "  Start day-to-day stack (see README Desktop 手通し / client/env.desktop.example)." >&2
  return 1
}

wait_tcp() {
  local endpoint="$1"
  local host port
  host="$(printf '%s' "${endpoint}" | sed -n 's|^tcp/\([^:]*\):\([0-9]*\)$|\1|p')"
  port="$(printf '%s' "${endpoint}" | sed -n 's|^tcp/\([^:]*\):\([0-9]*\)$|\2|p')"
  if [[ -z "${host}" || -z "${port}" ]]; then
    echo "smoke-desktop-handthrough: FAILED — bad ZENOH_CONNECT=${endpoint}" >&2
    return 1
  fi
  local deadline=$((SECONDS + HEALTH_WAIT_SECS))
  echo "smoke-desktop-handthrough: waiting for zenohd ${host}:${port} …"
  while (( SECONDS < deadline )); do
    if (echo >/dev/tcp/"${host}"/"${port}") >/dev/null 2>&1; then
      echo "smoke-desktop-handthrough: zenohd ok"
      return 0
    fi
    if command -v nc >/dev/null 2>&1 && nc -z "${host}" "${port}" >/dev/null 2>&1; then
      echo "smoke-desktop-handthrough: zenohd ok"
      return 0
    fi
    sleep 1
  done
  echo "smoke-desktop-handthrough: FAILED — zenohd not accepting ${endpoint}" >&2
  return 1
}

if ! command -v curl >/dev/null 2>&1; then
  echo "smoke-desktop-handthrough: FAILED — curl required" >&2
  exit 1
fi

wait_http "${AUTH_URL}/health" "auth"
wait_http "${WORLD_URL}/health" "world"
wait_http "${ASSETS_URL}/health" "assets"
wait_tcp "${ZENOH_CONNECT}"

echo "smoke-desktop-handthrough: building host-server + client …"
(
  cd "${ROOT}/client"
  cargo build -p host-server -p client
)

echo "smoke-desktop-handthrough: client auth / Home units …"
(
  cd "${ROOT}/client"
  cargo test -q -p client -- \
    auth::tests::auth_gate_clear_resets_join \
    system_ui::tests::home_tab_has_submit_and_bottom_tab \
    system_ui::dash::tests::open_home_tab_has_submit \
    --exact
)

rm -f "${HOST_READY_FILE}"
: >"${HOST_LOG}"
echo "smoke-desktop-handthrough: starting host-server (log ${HOST_LOG}) …"
export AUTH_URL WORLD_URL ZENOH_CONNECT ROOM_ID AUTH_USER AUTH_PASSWORD
export ALCHEMY_HOST_READY_FILE="${HOST_READY_FILE}"
(
  cd "${ROOT}/client"
  # Fixed info: banner greps fail under warn/error (same footgun as x001f).
  RUST_LOG=info cargo run -q -p host-server
) >"${HOST_LOG}" 2>&1 &
HOST_PID=$!

host_deadline=$((SECONDS + HOST_READY_SECS))
while (( SECONDS < host_deadline )); do
  if ! kill -0 "${HOST_PID}" 2>/dev/null; then
    echo "smoke-desktop-handthrough: FAILED — host-server exited early" >&2
    cat "${HOST_LOG}" >&2 || true
    exit 1
  fi
  if [[ -f "${HOST_READY_FILE}" ]]; then
    echo "smoke-desktop-handthrough: host-server ready (ready file)"
    break
  fi
  sleep 1
done
if [[ ! -f "${HOST_READY_FILE}" ]]; then
  echo "smoke-desktop-handthrough: FAILED — host-server ready file not written within ${HOST_READY_SECS}s" >&2
  cat "${HOST_LOG}" >&2 || true
  exit 1
fi

# Physics stats log every ~1s; ready file can land before the first line.
physics_deadline=$((SECONDS + 10))
while (( SECONDS < physics_deadline )); do
  if grep -q "physics: steps=" "${HOST_LOG}"; then
    break
  fi
  if ! kill -0 "${HOST_PID}" 2>/dev/null; then
    echo "smoke-desktop-handthrough: FAILED — host-server exited before physics stats" >&2
    cat "${HOST_LOG}" >&2 || true
    exit 1
  fi
  sleep 0.5
done

ok=1
if ! grep -q "auth JWT + host Room Token acquired and verified" "${HOST_LOG}"; then
  echo "smoke-desktop-handthrough: missing Room Token verified banner" >&2
  ok=0
fi
if ! grep -q "published SceneManifest revision=" "${HOST_LOG}"; then
  echo "smoke-desktop-handthrough: missing SceneManifest publish" >&2
  ok=0
fi
if ! grep -q "physics: steps=" "${HOST_LOG}"; then
  echo "smoke-desktop-handthrough: missing physics stats" >&2
  ok=0
fi
if ! grep -q "protocol_pin=alchemy-protocol/" "${HOST_LOG}"; then
  echo "smoke-desktop-handthrough: missing PROTOCOL_PIN" >&2
  ok=0
fi

if [[ "$ok" -ne 1 ]]; then
  echo "smoke-desktop-handthrough: FAILED — host log asserts" >&2
  cat "${HOST_LOG}" >&2 || true
  exit 1
fi

echo "smoke-desktop-handthrough: ok (host Token/Manifest/physics + client build/units; no Desktop GUI)"
