# Entry points (macOS / Linux / WSL).
.PHONY: ci ci-rust ci-protocol ci-auth ci-world help

help:
	@echo "Targets:"
	@echo "  make ci            # rust + protocol + auth + world (GHA equivalent)"
	@echo "  make ci-rust"
	@echo "  make ci-protocol"
	@echo "  make ci-auth       # Docker Compose (same on Win/Mac/Linux)"
	@echo "  make ci-world      # Docker Compose"
	@echo ""
	@echo "Dev (Slice B): see README.md"

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
