# Entry points (macOS / Linux / WSL).
.PHONY: ci ci-rust ci-protocol ci-auth ci-world ci-assets \
	smoke-assets smoke-assets-r2 help

help:
	@echo "Targets:"
	@echo "  make ci            # rust + protocol + auth + world + assets"
	@echo "  make ci-rust"
	@echo "  make ci-protocol"
	@echo "  make ci-auth"
	@echo "  make ci-world"
	@echo "  make ci-assets"
	@echo "  make smoke-assets      # local CAS API smoke (auth+assets up)"
	@echo "  make smoke-assets-r2   # x001c R2 CAS API smoke (ASSETS_CAS_BACKEND=s3)"
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
