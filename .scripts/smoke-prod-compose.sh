#!/usr/bin/env bash
# x003b: Validate prod compose S2S wiring; optionally bring up auth/world to /health.
#
# Default: compose config only (fast; CI-friendly).
# Full boot: SMOKE_PROD_UP=1 ./.scripts/smoke-prod-compose.sh
#
# Requires: docker compose, openssl (for ephemeral secrets when UP=1 or when
# generating a throwaway env for config).
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT"

AUTH_COMPOSE="auth-server/docker-compose.prod.yml"
WORLD_COMPOSE="world-server/docker-compose.prod.yml"
ASSETS_COMPOSE="assets-server/docker-compose.prod.yml"
TMP_DIR="${TMPDIR:-/tmp}/alchemy-smoke-prod-$$"
AUTH_ENV="${TMP_DIR}/auth.env.prod"
WORLD_ENV="${TMP_DIR}/world.env.prod"
ASSETS_ENV="${TMP_DIR}/assets.env.prod"
JWT_PEM="${TMP_DIR}/jwt_private.pem"
UP="${SMOKE_PROD_UP:-0}"
HEALTH_WAIT_SECS="${HEALTH_WAIT_SECS:-180}"

cleanup() {
  if [[ "${UP}" == "1" ]]; then
    docker compose -f "${AUTH_COMPOSE}" --env-file "${AUTH_ENV}" down --remove-orphans >/dev/null 2>&1 || true
    docker compose -f "${WORLD_COMPOSE}" --env-file "${WORLD_ENV}" down --remove-orphans >/dev/null 2>&1 || true
  fi
  rm -rf "${TMP_DIR}" 2>/dev/null || true
}
trap cleanup EXIT

mkdir -p "${TMP_DIR}"

SECRET_KEY_BASE="$(openssl rand -base64 48 | tr -d '\n')"
POSTGRES_PASSWORD="$(openssl rand -hex 16)"
AUTH_INTERNAL_TOKEN="$(openssl rand -hex 32)"

# Minimal RS256 PEM for auth release (config validation + optional UP).
openssl genrsa -out "${JWT_PEM}" 2048 >/dev/null 2>&1

cat >"${AUTH_ENV}" <<EOF
POSTGRES_PASSWORD=${POSTGRES_PASSWORD}
SECRET_KEY_BASE=${SECRET_KEY_BASE}
AUTH_FRONTEND_URL=http://127.0.0.1:4002
PHX_HOST=127.0.0.1
JWT_PRIVATE_KEY_FILE=${JWT_PEM}
AUTH_INTERNAL_TOKEN=${AUTH_INTERNAL_TOKEN}
TRUSTED_PROXIES=127.0.0.1,::1
EOF

cat >"${WORLD_ENV}" <<EOF
SECRET_KEY_BASE=${SECRET_KEY_BASE}
AUTH_JWKS_URL=http://host.docker.internal:4002/.well-known/jwks.json
AUTH_INTERNAL_URL=http://host.docker.internal:4002
AUTH_INTERNAL_TOKEN=${AUTH_INTERNAL_TOKEN}
EOF

cat >"${ASSETS_ENV}" <<EOF
SECRET_KEY_BASE=${SECRET_KEY_BASE}
AUTH_JWKS_URL=http://host.docker.internal:4002/.well-known/jwks.json
EOF

echo "smoke-prod-compose: docker compose config (auth)…"
docker compose -f "${AUTH_COMPOSE}" --env-file "${AUTH_ENV}" config >/dev/null
echo "smoke-prod-compose: auth config ok"

echo "smoke-prod-compose: docker compose config (world)…"
docker compose -f "${WORLD_COMPOSE}" --env-file "${WORLD_ENV}" config >/dev/null
echo "smoke-prod-compose: world config ok"

echo "smoke-prod-compose: docker compose config (assets)…"
docker compose -f "${ASSETS_COMPOSE}" --env-file "${ASSETS_ENV}" config >/dev/null
echo "smoke-prod-compose: assets config ok"

# Fail closed if S2S keys were dropped from the compose files.
auth_cfg="$(docker compose -f "${AUTH_COMPOSE}" --env-file "${AUTH_ENV}" config)"
world_cfg="$(docker compose -f "${WORLD_COMPOSE}" --env-file "${WORLD_ENV}" config)"
assets_cfg="$(docker compose -f "${ASSETS_COMPOSE}" --env-file "${ASSETS_ENV}" config)"
echo "${auth_cfg}" | grep -q 'AUTH_INTERNAL_TOKEN' || {
  echo "smoke-prod-compose: FAILED — auth prod compose missing AUTH_INTERNAL_TOKEN" >&2
  exit 1
}
echo "${world_cfg}" | grep -q 'AUTH_INTERNAL_TOKEN' || {
  echo "smoke-prod-compose: FAILED — world prod compose missing AUTH_INTERNAL_TOKEN" >&2
  exit 1
}
echo "${world_cfg}" | grep -q 'AUTH_INTERNAL_URL' || {
  echo "smoke-prod-compose: FAILED — world prod compose missing AUTH_INTERNAL_URL" >&2
  exit 1
}

# x0073: app port host publish defaults to loopback.
# Scope to the mapping that publishes $port — do not scan whole compose
# (auth also publishes Postgres on 127.0.0.1:5433; that must not mask 4002).
assert_loopback_publish() {
  local name="$1"
  local cfg="$2"
  local port="$3"
  local block
  block="$(printf '%s\n' "${cfg}" | grep -E -B6 -A3 "published: \"?${port}\"?")" || {
    echo "smoke-prod-compose: FAILED — ${name} prod compose missing published ${port}" >&2
    exit 1
  }
  echo "${block}" | grep -E "host_ip: 127\\.0\\.0\\.1" >/dev/null || {
    echo "smoke-prod-compose: FAILED — ${name} published ${port} is not bound to 127.0.0.1" >&2
    echo "${block}" >&2
    exit 1
  }
  if echo "${block}" | grep -E "host_ip: ['\"]?0\\.0\\.0\\.0['\"]?" >/dev/null; then
    echo "smoke-prod-compose: FAILED — ${name} published ${port} still has host_ip 0.0.0.0" >&2
    echo "${block}" >&2
    exit 1
  fi
}
assert_loopback_publish "auth" "${auth_cfg}" "4002"
assert_loopback_publish "world" "${world_cfg}" "4003"
assert_loopback_publish "assets" "${assets_cfg}" "4004"
echo "smoke-prod-compose: loopback publish ok (auth/world/assets)"

if [[ "${UP}" != "1" ]]; then
  echo "smoke-prod-compose: config-only pass (set SMOKE_PROD_UP=1 for /health boot)"
  exit 0
fi

wait_http() {
  local url="$1"
  local name="$2"
  local deadline=$((SECONDS + HEALTH_WAIT_SECS))
  echo "smoke-prod-compose: waiting for ${name} at ${url} …"
  while (( SECONDS < deadline )); do
    if curl -sf --max-time 3 "${url}" >/dev/null 2>&1; then
      echo "smoke-prod-compose: ${name} ok"
      return 0
    fi
    sleep 2
  done
  echo "smoke-prod-compose: FAILED — ${name} not healthy within ${HEALTH_WAIT_SECS}s (${url})" >&2
  return 1
}

echo "smoke-prod-compose: building/up auth prod…"
docker compose -f "${AUTH_COMPOSE}" --env-file "${AUTH_ENV}" up -d --build
wait_http "http://127.0.0.1:4002/health" "auth"

echo "smoke-prod-compose: migrate auth…"
docker compose -f "${AUTH_COMPOSE}" --env-file "${AUTH_ENV}" exec -T auth \
  bin/auth eval 'Auth.Release.migrate()' >/dev/null

echo "smoke-prod-compose: building/up world prod…"
docker compose -f "${WORLD_COMPOSE}" --env-file "${WORLD_ENV}" up -d --build
wait_http "http://127.0.0.1:4003/health" "world"

echo "smoke-prod-compose: PASS (auth + world /health)"
