# Entry points (macOS / Linux / WSL).
.PHONY: ci ci-rust ci-protocol ci-auth ci-world ci-assets help

help:
	@echo "Targets:"
	@echo "  make ci            # rust + protocol + auth + world + assets"
	@echo "  make ci-rust"
	@echo "  make ci-protocol"
	@echo "  make ci-auth"
	@echo "  make ci-world"
	@echo "  make ci-assets"
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
