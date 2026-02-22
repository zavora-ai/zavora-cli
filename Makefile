.PHONY: fmt fmt-check check lint test eval quality-gate security-check perf-check ci release-check npm-pack-check brew-formula dist-check version-check

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

eval:
	cargo run -- eval run --dataset evals/datasets/retrieval-baseline.v1.json --output evals/reports/latest.json --benchmark-iterations 200 --fail-under 0.90

quality-gate:
	./scripts/quality_gate.sh

security-check:
	./scripts/security_check.sh

perf-check:
	./scripts/perf_reliability.sh

ci: fmt-check check lint test quality-gate security-check

release-check: ci security-check
	@echo "Release preflight checks passed."

npm-pack-check:
	cd npm/zavora-cli && npm pack --dry-run >/dev/null

version-check:
	@CARGO_VERSION="$$(awk -F'\"' '/^version = / { print $$2; exit }' Cargo.toml)"; \
	NPM_VERSION="$$(node -e 'console.log(require("./npm/zavora-cli/package.json").version)')"; \
	test "$$CARGO_VERSION" = "$$NPM_VERSION" || \
	( echo \"Version mismatch: Cargo.toml=$$CARGO_VERSION npm/zavora-cli/package.json=$$NPM_VERSION\" >&2; exit 1 )

brew-formula:
	./scripts/generate_homebrew_formula.sh

dist-check: release-check version-check npm-pack-check
	@echo "Distribution checks passed."
