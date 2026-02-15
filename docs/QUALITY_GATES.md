# Quality Gates

This document defines the CI/release quality gate path for Sprint 5.

## Gate Command

Run locally or in CI:

```bash
make quality-gate
```

This executes:

1. Eval harness threshold gate (`zavora-cli eval run`)
2. Guardrail regression tests (`cargo test guardrail_`)
3. Security hardening gate (`make security-check`) during release workflow
4. RustSec advisory policy tracked in `.cargo/audit.toml` (temporary ignores must be reviewed each release)

## Thresholds

- Eval dataset: `evals/datasets/retrieval-baseline.v1.json`
- Default fail-under threshold: `0.90`
- Benchmark iterations: `200`

Override in CI/local shell when needed:

- `ZAVORA_EVAL_FAIL_UNDER`
- `ZAVORA_EVAL_BENCH_ITERATIONS`

## Actionable Failure Guidance

When the gate fails:

1. Read the error block emitted by `scripts/quality_gate.sh`.
2. Inspect generated eval report JSON (`.zavora/evals/ci-gate.json` by default).
3. Compare against baseline in `docs/EVAL_BASELINE.md`.
4. Fix regressions; only update baseline/dataset when behavior changes are intentional.
