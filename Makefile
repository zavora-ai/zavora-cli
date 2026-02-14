.PHONY: fmt fmt-check check lint test ci release-check

fmt:
	cargo fmt --all

fmt-check:
	cargo fmt --all --check

check:
	cargo check

lint:
	cargo clippy --all-targets -- -D warnings

test:
	cargo test

ci: fmt-check check lint test

release-check: ci
	@echo "Release preflight checks passed."
