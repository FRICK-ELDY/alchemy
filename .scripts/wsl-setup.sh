#!/usr/bin/env bash
# Ubuntu 26.04 (WSL2) 向け: Alchemy Slice A の開発ツールを入れる。
# 使い方（WSL 内）:
#   cd /mnt/d/github/frick/alchemy   # パスは環境に合わせる
#   bash .scripts/wsl-setup.sh
set -euo pipefail

echo "==> apt packages (build / wgpu / docker CLI peers)"
sudo apt-get update
sudo apt-get install -y \
  build-essential pkg-config curl ca-certificates git \
  libxkbcommon-dev libwayland-dev \
  libx11-dev libxcursor-dev libxi-dev libxrandr-dev

if ! command -v rustup >/dev/null 2>&1; then
  echo "==> rustup"
  curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh -s -- -y
  # shellcheck disable=SC1091
  source "$HOME/.cargo/env"
else
  echo "==> rustup already installed"
  # shellcheck disable=SC1091
  source "$HOME/.cargo/env" 2>/dev/null || true
fi

rustup default stable
rustup component add rustfmt clippy

if ! command -v zenohd >/dev/null 2>&1; then
  echo "==> zenohd (cargo install)"
  cargo install zenohd
else
  echo "==> zenohd already on PATH"
fi

echo ""
echo "Setup done. Next:"
echo "  cd <alchemy-repo>"
echo "  chmod +x .scripts/ci.sh && make ci-rust"
echo "  # Docker: Docker Desktop の Settings > Resources > WSL Integration でこの distro を有効化"
echo "  cd auth-server && docker compose up -d"
