#!/usr/bin/env bash
set -euo pipefail

DATASET_PATH="${1:-evals/datasets/retrieval-baseline.v1.json}"
OUTPUT_PATH="${2:-.zavora/evals/ci-gate.json}"
BENCH_ITERATIONS="${ZAVORA_EVAL_BENCH_ITERATIONS:-200}"
FAIL_UNDER="${ZAVORA_EVAL_FAIL_UNDER:-0.90}"

echo "[QUALITY_GATE] Running eval harness..."
if ! cargo run -- eval run \
  --dataset "${DATASET_PATH}" \
  --output "${OUTPUT_PATH}" \
  --benchmark-iterations "${BENCH_ITERATIONS}" \
  --fail-under "${FAIL_UNDER}"; then
  cat >&2 <<'MSG'
[QUALITY_GATE] Eval gate failed.
Action:
1. Inspect the eval report JSON for failing cases.
2. Compare failures against docs/EVAL_BASELINE.md.
3. Fix regressions or intentionally version/update the dataset baseline.
MSG
  exit 1
fi

echo "[QUALITY_GATE] Running guardrail regression tests..."
if ! cargo test guardrail_ -- --nocapture; then
  cat >&2 <<'MSG'
[QUALITY_GATE] Guardrail gate failed.
Action:
1. Review guardrail test failures.
2. Ensure block/redact/observe behavior remains deterministic.
3. Re-run cargo test guardrail_ after applying fixes.
MSG
  exit 1
fi

echo "[QUALITY_GATE] Passed."
