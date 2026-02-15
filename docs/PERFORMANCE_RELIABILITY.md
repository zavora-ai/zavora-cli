# Performance And Reliability

This document defines GA performance targets and the repeatable validation flow for Sprint 6 (`#26`).

## Targets

- Eval benchmark p95 latency: `<= 5 ms`
- `/healthz` p95 latency under load: `<= 50 ms`
- `/v1/a2a/ping` p95 latency under load: `<= 80 ms`
- Server steady-state RSS during stress run: `<= 300 MB`

Load profile defaults:
- `300` requests per endpoint
- concurrency `30`

## Runbook

Run the full performance/reliability harness:

```bash
make perf-check
```

Artifacts:
- Eval report: `.zavora/evals/perf-reliability.json`
- Perf summary: `.zavora/perf/latest.json`

Override knobs:
- `PERF_REQUESTS`
- `PERF_CONCURRENCY`
- `PERF_EVAL_BENCH_ITERATIONS`
- `PERF_EVAL_P95_TARGET_MS`
- `PERF_HEALTH_P95_TARGET_MS`
- `PERF_A2A_P95_TARGET_MS`
- `PERF_SERVER_RSS_TARGET_MB`

## Bottlenecks Addressed Before GA

- Server ask path no longer rebuilds ADK runners for repeated `(user_id, session_id)` pairs; server mode now caches runners and reuses initialized session services.
- Added endpoint stress harness to catch latency regressions on `/healthz` and `/v1/a2a/ping` before release cut.
- Added explicit threshold enforcement with machine-readable summary output for release review.

## Latest Summary

Latest committed run: **February 15, 2026** (`evals/reports/perf-reliability-baseline.v1.json`)

- Eval benchmark: `p95=0.011 ms`, `throughput=106246.146 qps`
- `/healthz` load: `p95=9.597 ms` (`300` requests, concurrency `30`)
- `/v1/a2a/ping` load: `p95=19.277 ms` (`300` requests, concurrency `30`)
- Server RSS during stress: `20.16 MB`

Working report path for iterative runs remains `.zavora/perf/latest.json`.
