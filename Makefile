.PHONY: fmt fmt-check check lint test eval quality-gate security-check perf-check ci release-check

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
