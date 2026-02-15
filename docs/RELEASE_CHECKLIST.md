# Release Checklist

Run this checklist before creating a `vX.Y.Z` tag.

## Code And Quality

- [ ] Scope matches planned release objective
- [ ] `cargo fmt --all --check`
- [ ] `cargo check`
- [ ] `cargo clippy --all-targets -- -D warnings`
- [ ] `cargo test`
- [ ] `make quality-gate`
- [ ] `make security-check`
- [ ] `.cargo/audit.toml` reviewed; temporary ignored advisories still justified
- [ ] `cargo run -- eval run --dataset evals/datasets/retrieval-baseline.v1.json --output .zavora/evals/release-candidate.json --benchmark-iterations 200 --fail-under 0.90`
- [ ] Eval report reviewed and baseline updated in `docs/EVAL_BASELINE.md` (when release baseline changes)
- [ ] Guardrail regression tests reviewed (`cargo test guardrail_`)

## Release Notes

- [ ] `CHANGELOG.md` updated
- [ ] Known risks and mitigations documented
- [ ] Rollback plan documented

## Documentation

- [ ] `README.md` quickstart and command examples reflect current CLI behavior
- [ ] Sprint/release docs reviewed: `docs/PROJECT_PLAN.md`, `docs/GITHUB_MILESTONE_ISSUES.md`
- [ ] Session retention and lifecycle command docs updated when relevant (`sessions list/show/delete/prune`)

## Tag And Publish

- [ ] Tag created: `vX.Y.Z`
- [ ] Tag pushed
- [ ] GitHub release artifacts generated
