# Working Style

This document defines how Zavora CLI work is executed from Sprint 7 onward.

## Core Principles

- Ship in release slices, not big-bang rewrites.
- Close issues only with runnable evidence.
- Favor deterministic behavior over implicit magic.
- Keep security defaults strict; privileged behavior must be explicit.
- Preserve user trust: clear prompts, clear errors, clear rollback paths.

## Planning And Execution

- Every sprint has:
  - objective
  - issue list with acceptance criteria
  - explicit quality/security/perf gates
- Every issue should be independently shippable where possible.
- Use labels for routing and reporting:
  - `priority:*`
  - `sprint:*`
  - `area:*`
  - `phase:2`
  - `track:q-cli-parity`

## Engineering Rules

- Prefer small PRs tied to one issue.
- Keep changes backward-compatible unless migration notes are included.
- Add tests for behavior changes and policy logic.
- Avoid exposing sensitive config values in user-facing output by default.
- Prefer explicit feature toggles for experimental behavior.

## Definition Of Done (Issue)

- Acceptance criteria are all satisfied.
- Tests added/updated and passing locally.
- Relevant docs updated.
- Evidence posted in issue comment:
  - command(s) run
  - key output
  - commit hash
- Issue and project status updated.

## Release Gates

Minimum release checks:

- `make release-check`
- `make perf-check`

Additional checks when relevant:

- `cargo test --features semantic-search`
- feature-specific smoke commands (for example server/MCP/chat mode checks)

## Security And Privacy Baseline

- Do not print secrets or sensitive paths unless explicitly requested by privileged flags.
- Keep guardrail and tool-approval defaults safe.
- Keep audit/policy artifacts current (`.cargo/audit.toml`, security docs, release checklist).

## Communication Style

- Findings first, then summary.
- State assumptions explicitly.
- Use concrete dates and version numbers in plans/status reports.
- If a command or automation behaves unexpectedly, report it and correct it before continuing.
