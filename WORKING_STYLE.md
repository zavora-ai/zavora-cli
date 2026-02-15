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
- Add doc comments for all public types and functions.
- Reuse existing patterns from ADK-Rust and reference implementations before inventing new ones.
- Keep token/context budget awareness in design — prefer char-based heuristics over external tokenizer dependencies.
- Experimental features must be gated and clearly labeled in user-facing output.
- Prefer simple state machines over complex orchestration for conversation features (tangent, checkpoint).

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

## Reference Projects

When implementing new features, consult these reference projects for proven patterns:

- **ADK-Rust** (`~/Developer/projects/adk-rust`): Core agent/tool/session abstractions. Use its traits and patterns directly.
- **Amazon Q Developer CLI** (`~/Developer/reference/amazon-q-developer-cli`): Chat UX, hooks, checkpoints, tangent mode, todos, context management, token counting. Adapt patterns to zavora-cli's architecture without copying verbatim.
