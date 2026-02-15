# Agile Release Cycle

This repository uses a release-based agile cycle optimized for small, shippable increments.

## Cadence

Use a fixed cycle (example: 1-2 weeks) per release:

1. `Plan` (Day 1)
2. `Build` (Days 2-N)
3. `Harden` (last 1-2 days)
4. `Release` (tag + notes)
5. `Learn` (post-release review)

## Release Unit

Each release must include:

- Clear objective
- Explicit in-scope / out-of-scope
- Observable user outcome
- Validation criteria (tests, checks, manual verification)
- Rollback notes

## Workflow

1. Create a release objective and list candidate slices.
2. Prioritize smallest independent slice first.
3. Implement continuously with CI green.
4. Keep feature changes mapped to the current release objective.
5. Freeze scope near end of cycle; only bugfixes and risk removals.
6. Tag `vX.Y.Z` and publish release notes.

## Quality Gates

Minimum gates before tagging:

- `cargo fmt --check`
- `cargo check`
- `cargo clippy --all-targets -- -D warnings`
- `cargo test`
- `cargo run -- eval run --dataset evals/datasets/retrieval-baseline.v1.json --output .zavora/evals/release-candidate.json --benchmark-iterations 200 --fail-under 0.90`
- Changelog entry prepared

## Branch + Tag Strategy

- Mainline development on `main` (or short-lived feature branches)
- Optional release stabilization branch: `release/vX.Y.Z`
- Release tags are immutable: `vX.Y.Z`

## Definition Of Done (Per Release)

- Scope completed
- Tests and checks passing
- Operational risks documented
- Upgrade notes written
- Tag + changelog published
