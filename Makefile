# Cross-platform entry points (macOS / Linux / WSL). Windows: use .scripts/ci.ps1 or Git Bash.
.PHONY: ci ci-rust ci-protocol ci-auth help

help:
	@echo "Targets:"
	@echo "  make ci            # rust + protocol + auth (GHA equivalent)"
	@echo "  make ci-rust"
	@echo "  make ci-protocol"
	@echo "  make ci-auth       # Docker Compose (same on Win/Mac/Linux)"
	@echo ""
	@echo "Dev (Slice A): see README.md"

ci:
	./.scripts/ci.sh all

ci-rust:
	./.scripts/ci.sh rust

ci-protocol:
	./.scripts/ci.sh protocol

ci-auth:
	./.scripts/ci.sh auth
