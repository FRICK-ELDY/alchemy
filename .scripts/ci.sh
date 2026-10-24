#!/usr/bin/env bash
# GitHub Actions 相当のチェック（macOS / Linux / WSL / Git Bash）。
# 使い方: ./.scripts/ci.sh [all|rust|protocol|auth|world|assets]
#
# Elixir 系は既定で Docker（OS 差分・ネイティブ依存を避ける）。
# ホストの mix で auth を回す場合: AUTH_CI_HOST=1 ./.scripts/ci.sh auth
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
  step "cargo build --workspace" cargo build --workspace
  step "cargo test --workspace" cargo test --workspace
  popd >/dev/null
}

run_protocol() {
  pushd protocol >/dev/null
  step "cargo test (protocol)" cargo test
  popd >/dev/null
  if command -v protoc >/dev/null 2>&1; then
    out="${TMPDIR:-/tmp}/alchemy-descriptor.pb"
    step "protoc schema compile" \
      bash -c "protoc -I protocol/proto --include_imports --descriptor_set_out='${out}' protocol/proto/render_frame.proto protocol/proto/input_events.proto && test -s '${out}'"
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
    auth sh -c 'mix deps.get && mix format --check-formatted && mix compile --warnings-as-errors && mix credo --strict && mix ecto.create && mix ecto.migrate && mix test'
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
  (
    cd auth-server
    step "mix deps.get" mix deps.get
    step "mix format --check-formatted" mix format --check-formatted
    step "mix compile --warnings-as-errors" mix compile --warnings-as-errors
    step "mix credo --strict" mix credo --strict
    step "mix ecto.create" mix ecto.create
    step "mix ecto.migrate" mix ecto.migrate
    step "mix test" mix test
  )
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
  step "world mix checks (Docker)" docker compose -f "$compose" run --rm --entrypoint "" \
    -e MIX_ENV=test \
    world sh -c 'mix deps.get && mix format --check-formatted && mix compile --warnings-as-errors && mix credo --strict && mix test'
}

run_assets_docker() {
  local compose="assets-server/docker-compose.yml"
  if ! command -v docker >/dev/null 2>&1; then
    echo "[FAIL] docker is required for assets CI" >&2
    FAILED+=("docker required")
    return
  fi
  step "docker compose build assets" docker compose -f "$compose" build assets
  step "assets mix checks (Docker)" docker compose -f "$compose" run --rm --entrypoint "" \
    -e MIX_ENV=test \
    assets sh -c 'mix deps.get && mix format --check-formatted && mix compile --warnings-as-errors && mix credo --strict && mix test'
}

echo "Alchemy local CI (target=${TARGET})"
echo "root: ${ROOT}"

case "$TARGET" in
  rust) run_rust ;;
  protocol) run_protocol ;;
  auth) run_auth ;;
  world) run_world_docker ;;
  assets) run_assets_docker ;;
  all)
    run_rust
    run_protocol
    run_auth
    run_world_docker
    run_assets_docker
    ;;
  *)
    echo "usage: $0 [all|rust|protocol|auth|world|assets]" >&2
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
