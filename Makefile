.PHONY: fmt fmt-check check lint test eval quality-gate security-check perf-check ci release-check npm-pack-check dist-check version-check local-adk unlink-adk clean-clone-check docs-check wiring-check audit feature-matrix

local-adk:
	cp .cargo/config.toml.local-adk .cargo/config.toml
	@echo "Local ADK-Rust override installed. Run 'make unlink-adk' to build against crates.io again."

unlink-adk:
	rm -f .cargo/config.toml
	@echo "Local ADK-Rust override removed; building against crates.io."

fmt:
	cargo fmt --all

fmt-check:
	cargo fmt --all --check

check:
	cargo check --all-targets

feature-matrix:
	cargo check --all-targets --features "web-fetch,lsp,oauth,browser,sandbox,rag,semantic-search,checkpoints"

lint:
	cargo clippy --all-targets -- -D warnings

test:
	cargo test --all-targets -- --test-threads=1

audit:
	cargo audit

clean-clone-check:
	./scripts/check_clean_clone.sh

docs-check:
	./scripts/check_docs.sh

wiring-check:
	./scripts/check_wiring.sh

eval:
	cargo run -- eval run --dataset evals/datasets/retrieval-baseline.v1.json --output evals/reports/latest.json --benchmark-iterations 200 --fail-under 0.90

quality-gate:
	./scripts/quality_gate.sh

security-check:
	./scripts/security_check.sh

perf-check:
	./scripts/perf_reliability.sh

ci: fmt-check check feature-matrix lint test wiring-check docs-check quality-gate security-check audit

release-check: ci clean-clone-check version-check
	cargo publish --dry-run --locked
	@echo "Release preflight checks passed."

npm-pack-check:
	cd npm/zavora-cli && npm pack --dry-run >/dev/null

# The Homebrew formula is no longer checked here. It lives in
# zavora-ai/homebrew-tap, because Homebrew 6 will not install a formula from a URL
# or a loose file, and it is pinned there by the release tag itself — so its version
# cannot disagree with the tag by construction. Checking a copy in this repository
# only asserted that an unused file was consistent.
version-check:
	@CARGO_VERSION="$$(awk -F'\"' '/^version = / { print $$2; exit }' Cargo.toml)"; \
	NPM_VERSION="$$(node -e 'console.log(require("./npm/zavora-cli/package.json").version)')"; \
	test "$$CARGO_VERSION" = "$$NPM_VERSION" || \
	( echo "Version mismatch: Cargo.toml=$$CARGO_VERSION npm/zavora-cli/package.json=$$NPM_VERSION" >&2; exit 1 ); \
	echo "Versions agree: $$CARGO_VERSION"

dist-check: release-check npm-pack-check
	@echo "Distribution checks passed."
