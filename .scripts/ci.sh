#!/usr/bin/env bash
# GitHub Actions 相当のチェック（macOS / Linux / WSL / Git Bash）。
# 使い方: ./.scripts/ci.sh [all|rust|relay|protocol|auth|world|assets|shared|prod-compose]
#
# Elixir 系は既定で Docker（OS 差分・ネイティブ依存を避ける）。
# ホストの mix で auth を回す場合: AUTH_CI_HOST=1 ./.scripts/ci.sh auth
# ホストの mix で shared/alchemy_* を回す場合: SHARED_CI_HOST=1 ./.scripts/ci.sh shared
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT"
TARGET="${1:-all}"
FAILED=()

step() {
  local name="$1"
  shift
  echo ""
  echo "=== ${name} ==="
  if "$@"; then
    echo "[PASS] ${name}"
  else
    echo "[FAIL] ${name}"
    FAILED+=("${name}")
  fi
}

run_rust() {
  pushd client >/dev/null
  step "cargo fmt --check" cargo fmt --all -- --check
  step "cargo clippy" cargo clippy --workspace -- -D warnings
  # Local may SKIP; GitHub Actions rust-deny job always gates (x002d).
  if command -v cargo-deny >/dev/null 2>&1; then
    step "cargo deny check" cargo deny check
  else
    echo "[SKIP] cargo deny check (local only; CI gates via EmbarkStudios/cargo-deny-action — install: cargo install cargo-deny)"
  fi
  step "cargo build --workspace" cargo build --workspace
  step "cargo test --workspace" cargo test --workspace
  popd >/dev/null
}

run_relay_agent() {
  # x0068 / P1-1: alchemy-relay-agent is outside the client workspace (parity with
  # .github/workflows/rust-ci.yml relay-agent-check).
  pushd relay-agent >/dev/null
  step "relay-agent cargo fmt --check" cargo fmt --all -- --check
  step "relay-agent cargo clippy" cargo clippy -- -D warnings
  if command -v cargo-deny >/dev/null 2>&1; then
    step "relay-agent cargo deny check" cargo deny check
  else
    echo "[SKIP] relay-agent cargo deny check (local only; CI gates via EmbarkStudios/cargo-deny-action)"
  fi
  step "relay-agent cargo build" cargo build
  step "relay-agent cargo test" cargo test
  popd >/dev/null
}

run_protocol() {
  pushd protocol >/dev/null
  step "cargo test (protocol)" cargo test
  popd >/dev/null
  if command -v protoc >/dev/null 2>&1; then
    out="${TMPDIR:-/tmp}/alchemy-descriptor.pb"
    step "protoc schema compile" \
      bash -c "protoc -I protocol/proto --include_imports --descriptor_set_out='${out}' protocol/proto/render_frame.proto protocol/proto/input_events.proto protocol/proto/presence.proto protocol/proto/scene_manifest.proto && test -s '${out}'"
  else
    echo "[SKIP] protoc schema compile (protoc not on PATH; optional locally)"
  fi
}

run_auth_docker() {
  local compose="auth-server/docker-compose.yml"
  if ! command -v docker >/dev/null 2>&1; then
    echo "[FAIL] docker is required for auth CI (or set AUTH_CI_HOST=1 with local Elixir)" >&2
    FAILED+=("docker required")
    return
  fi
  step "docker compose up postgres" docker compose -f "$compose" up -d postgres
  step "docker compose build auth" docker compose -f "$compose" build auth
  step "auth mix checks (Docker)" docker compose -f "$compose" run --rm --entrypoint "" \
    -e MIX_ENV=test -e PGHOST=postgres -e PGPORT=5432 \
    auth sh -c 'mix deps.get && mix hex.audit && mix format --check-formatted && mix compile --warnings-as-errors && mix credo --strict && mix ecto.create && mix ecto.migrate && mix test'
}

run_auth_host() {
  if ! command -v mix >/dev/null 2>&1; then
    echo "[FAIL] mix not found (install Elixir 1.19 / OTP 28, or unset AUTH_CI_HOST)" >&2
    FAILED+=("mix not found")
    return
  fi
  export MIX_ENV=test
  export PGPORT="${PGPORT:-5433}"
  export PGHOST="${PGHOST:-localhost}"
  # pushd（not subshell): step mutates FAILED in this shell.
  pushd auth-server >/dev/null
  step "mix deps.get" mix deps.get
  step "mix hex.audit" mix hex.audit
  step "mix format --check-formatted" mix format --check-formatted
  step "mix compile --warnings-as-errors" mix compile --warnings-as-errors
  step "mix credo --strict" mix credo --strict
  step "mix ecto.create" mix ecto.create
  step "mix ecto.migrate" mix ecto.migrate
  step "mix test" mix test
  popd >/dev/null
}

run_auth() {
  if [[ "${AUTH_CI_HOST:-}" == "1" ]]; then
    run_auth_host
  else
    run_auth_docker
  fi
}

run_world_docker() {
  local compose="world-server/docker-compose.yml"
  if ! command -v docker >/dev/null 2>&1; then
    echo "[FAIL] docker is required for world CI" >&2
    FAILED+=("docker required")
    return
  fi
  step "docker compose build world" docker compose -f "$compose" build world
  # x003b: force MIX_ENV=test; runtime.exs ignores WORLD_DATABASE_PATH in test
  # (assets CAS isolation と同型). Non-empty pollution path proves isolation test.
  step "world mix checks (Docker)" docker compose -f "$compose" run --rm --entrypoint "" \
    -e MIX_ENV=test \
    -e WORLD_DATABASE_PATH=/tmp/alchemy-world-test-must-ignore.db \
    world sh -c 'mix deps.get && mix hex.audit && mix format --check-formatted && mix compile --warnings-as-errors && mix credo --strict && mix test'
}

run_prod_compose_config() {
  if ! command -v docker >/dev/null 2>&1; then
    echo "[SKIP] prod compose config (docker not available)"
    return
  fi
  if ! command -v openssl >/dev/null 2>&1; then
    echo "[SKIP] prod compose config (openssl not available)"
    return
  fi
  step "prod compose config (x003b)" bash .scripts/smoke-prod-compose.sh
}

# x0089 / P2-9: parity with .github/workflows/alchemy-{auth-client,client-ip,rate-limit}-ci.yml
SHARED_PACKAGES=(alchemy_auth_client alchemy_client_ip alchemy_rate_limit)

run_shared_pkg_host() {
  local pkg="$1"
  # pushd（not subshell): step mutates FAILED in this shell.
  pushd "shared/${pkg}" >/dev/null
  step "shared/${pkg} mix deps.get" mix deps.get
  step "shared/${pkg} mix format --check-formatted" mix format --check-formatted
  step "shared/${pkg} mix compile --warnings-as-errors" mix compile --warnings-as-errors
  step "shared/${pkg} mix test" mix test
  popd >/dev/null
}

run_shared_host() {
  if ! command -v mix >/dev/null 2>&1; then
    echo "[FAIL] mix not found (install Elixir 1.19 / OTP 28, or unset SHARED_CI_HOST)" >&2
    FAILED+=("mix not found")
    return
  fi
  export MIX_ENV=test
  local pkg
  for pkg in "${SHARED_PACKAGES[@]}"; do
    run_shared_pkg_host "${pkg}"
  done
}

run_shared_docker() {
  if ! command -v docker >/dev/null 2>&1; then
    echo "[FAIL] docker is required for shared CI (or set SHARED_CI_HOST=1 with local Elixir)" >&2
    FAILED+=("docker required")
    return
  fi
  local image="${ALCHEMY_ELIXIR_IMAGE:-elixir:1.19-otp-28-alpine}"
  local host_shared="${ROOT}/shared"
  local -a docker_prefix=()
  local -a vol_args=()
  local pkg
  # Git Bash/MSYS rewrites bare /shared to %Git%/shared; disable conversion and pass a Windows host path.
  case "$(uname -s 2>/dev/null || true)" in
    MINGW*|MSYS*|CYGWIN*)
      host_shared="$(cd "${host_shared}" && pwd -W | sed 's|\\|/|g')"
      docker_prefix=(env MSYS_NO_PATHCONV=1)
      ;;
  esac
  # Bind sources only; hide deps/_build with named volumes (same pattern as auth/world/assets
  # compose) so host-built BEAM from SHARED_CI_HOST / IDE never shadow the Linux container.
  vol_args+=(-v "${host_shared}:/shared")
  for pkg in "${SHARED_PACKAGES[@]}"; do
    vol_args+=(-v "alchemy-ci-shared-${pkg}-deps:/shared/${pkg}/deps")
    vol_args+=(-v "alchemy-ci-shared-${pkg}-build:/shared/${pkg}/_build")
  done
  # Persist hex/rebar across runs; one container runs all packages (single apk + mix local).
  # Package list is SHARED_PACKAGES only (passed via env — do not hardcode a second list).
  vol_args+=(-v "alchemy-ci-shared-mix:/tmp/mix")
  vol_args+=(-v "alchemy-ci-shared-hex:/tmp/hex")
  step "shared/alchemy_* mix checks (Docker)" "${docker_prefix[@]}" docker run --rm \
    "${vol_args[@]}" \
    -w /shared \
    -e MIX_ENV=test \
    -e MIX_HOME=/tmp/mix \
    -e HEX_HOME=/tmp/hex \
    -e "ALCHEMY_SHARED_PACKAGES=${SHARED_PACKAGES[*]}" \
    "${image}" \
    sh -c 'set -eu
apk add --no-cache build-base git >/dev/null
mix local.hex --force >/dev/null
mix local.rebar --force >/dev/null
for pkg in $ALCHEMY_SHARED_PACKAGES; do
  echo "=== shared/${pkg} ==="
  cd "/shared/${pkg}"
  mix deps.get
  mix format --check-formatted
  mix compile --warnings-as-errors
  mix test
  cd /shared
done
'
}

run_shared() {
  if [[ "${SHARED_CI_HOST:-}" == "1" ]]; then
    run_shared_host
  else
    run_shared_docker
  fi
}

run_assets_docker() {
  local compose="assets-server/docker-compose.yml"
  if ! command -v docker >/dev/null 2>&1; then
    echo "[FAIL] docker is required for assets CI" >&2
    FAILED+=("docker required")
    return
  fi
  step "docker compose build assets" docker compose -f "$compose" build assets
  # x001e: force local CAS so developer assets-server/.env (R2) cannot affect mix test
  step "assets mix checks (Docker)" docker compose -f "$compose" run --rm --entrypoint "" \
    -e MIX_ENV=test \
    -e ASSETS_CAS_BACKEND=local \
    assets sh -c 'mix deps.get && mix hex.audit && mix format --check-formatted && mix compile --warnings-as-errors && mix credo --strict && mix test'
}

echo "Alchemy local CI (target=${TARGET})"
echo "root: ${ROOT}"

case "$TARGET" in
  rust) run_rust ;;
  relay|relay-agent) run_relay_agent ;;
  protocol) run_protocol ;;
  auth) run_auth ;;
  world) run_world_docker ;;
  assets) run_assets_docker ;;
  shared) run_shared ;;
  prod-compose) run_prod_compose_config ;;
  all)
    run_rust
    run_relay_agent
    run_protocol
    run_auth
    run_world_docker
    run_assets_docker
    run_shared
    run_prod_compose_config
    ;;
  *)
    echo "usage: $0 [all|rust|relay|protocol|auth|world|assets|shared|prod-compose]" >&2
    exit 2
    ;;
esac

echo ""
if [[ ${#FAILED[@]} -eq 0 ]]; then
  echo "RESULT: ALL PASSED"
  exit 0
fi
echo "RESULT: FAILED — [${FAILED[*]}]"
exit 1
