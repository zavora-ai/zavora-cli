.PHONY: fmt fmt-check check lint test eval quality-gate ci release-check

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

ci: fmt-check check lint test quality-gate

release-check: ci
	@echo "Release preflight checks passed."
