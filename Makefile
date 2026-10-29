# Entry points (macOS / Linux / WSL / Git Bash).
# pack-windows / publish-windows: native Windows Git Bash or windows-latest only.
.PHONY: ci ci-rust ci-launcher ci-protocol ci-auth ci-world ci-assets ci-e2e ci-prod-compose \
	smoke-assets smoke-assets-r2 smoke-compose-e2e smoke-desktop-handthrough \
	smoke-prod-compose pack-windows publish-windows help

help:
	@echo "Targets:"
	@echo "  make ci            # rust + launcher + protocol + auth + world + assets + prod-compose config"
	@echo "  make ci-rust"
	@echo "  make ci-launcher       # x0047 alchemy-launcher fmt/clippy/build/test"
	@echo "  make ci-protocol"
	@echo "  make ci-auth"
	@echo "  make ci-world"
	@echo "  make ci-assets"
	@echo "  make ci-prod-compose   # x003b prod compose config (S2S env)"
	@echo "  make ci-e2e            # x001f hermetic compose E2E (stack must be up, or COMPOSE_UP=1)"
	@echo "  make smoke-assets      # local CAS API smoke (auth+assets up)"
	@echo "  make smoke-assets-r2   # x001c/x001d R2 CAS + 302 presign smoke (BACKEND=s3)"
	@echo "  make smoke-compose-e2e # alias of ci-e2e"
	@echo "  make smoke-prod-compose # x003b config; SMOKE_PROD_UP=1 for auth/world /health"
	@echo "  make smoke-desktop-handthrough  # x0020 host Token/Manifest/physics + client build (no GUI; stop other host first)"
	@echo "  make pack-windows      # Windows x64 tester ZIP → dist/ (PowerShell; native Windows / CI)"
	@echo "  make publish-windows   # tag + push; CI creates Pre-release (CREATE_TAG=1 PUSH_TAG=1)"
	@echo ""
	@echo "Dev: see README.md"

ci:
	./.scripts/ci.sh all

ci-rust:
	./.scripts/ci.sh rust

ci-launcher:
	./.scripts/ci.sh launcher

ci-protocol:
	./.scripts/ci.sh protocol

ci-auth:
	./.scripts/ci.sh auth

ci-world:
	./.scripts/ci.sh world

ci-assets:
	./.scripts/ci.sh assets

ci-prod-compose:
	./.scripts/ci.sh prod-compose

smoke-assets:
	./.scripts/smoke-assets.sh

smoke-assets-r2:
	./.scripts/smoke-assets-r2.sh

ci-e2e smoke-compose-e2e:
	./.scripts/smoke-compose-e2e.sh

smoke-prod-compose:
	./.scripts/smoke-prod-compose.sh

smoke-desktop-handthrough:
	./.scripts/smoke-desktop-handthrough.sh

# PowerShell is the pack/publish entry on Windows (no Python / make required).
# VERSION=v0.0.0-dev.YYYYMMDDHHMM  or  make pack-windows ARGS='-SkipBuild'
pack-windows:
	powershell.exe -NoProfile -ExecutionPolicy Bypass -File ./.scripts/pack-windows-zip.ps1 $(ARGS)

# Canonical: CREATE_TAG=1 PUSH_TAG=1 → CI packs + gh release.
# ARGS='-CreateTag -PushTag'  or  ARGS='-CreateTag -PushTag -PackLocal'
publish-windows:
	powershell.exe -NoProfile -ExecutionPolicy Bypass -File ./.scripts/publish-windows-release.ps1 $(ARGS)
