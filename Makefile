# Entry points (macOS / Linux / WSL / Git Bash).
.PHONY: ci ci-rust ci-protocol ci-auth ci-world ci-assets ci-e2e \
	smoke-assets smoke-assets-r2 smoke-compose-e2e smoke-desktop-handthrough help

help:
	@echo "Targets:"
	@echo "  make ci            # rust + protocol + auth + world + assets"
	@echo "  make ci-rust"
	@echo "  make ci-protocol"
	@echo "  make ci-auth"
	@echo "  make ci-world"
	@echo "  make ci-assets"
	@echo "  make ci-e2e            # x001f hermetic compose E2E (stack must be up, or COMPOSE_UP=1)"
	@echo "  make smoke-assets      # local CAS API smoke (auth+assets up)"
	@echo "  make smoke-assets-r2   # x001c/x001d R2 CAS + 302 presign smoke (BACKEND=s3)"
	@echo "  make smoke-compose-e2e # alias of ci-e2e"
	@echo "  make smoke-desktop-handthrough  # x0020 host Token/Manifest/physics + client build (no GUI; stop other host first)"
	@echo ""
	@echo "Dev: see README.md"

ci:
	./.scripts/ci.sh all

ci-rust:
	./.scripts/ci.sh rust

ci-protocol:
	./.scripts/ci.sh protocol

ci-auth:
	./.scripts/ci.sh auth

ci-world:
	./.scripts/ci.sh world

ci-assets:
	./.scripts/ci.sh assets

smoke-assets:
	./.scripts/smoke-assets.sh

smoke-assets-r2:
	./.scripts/smoke-assets-r2.sh

ci-e2e smoke-compose-e2e:
	./.scripts/smoke-compose-e2e.sh

smoke-desktop-handthrough:
	./.scripts/smoke-desktop-handthrough.sh
