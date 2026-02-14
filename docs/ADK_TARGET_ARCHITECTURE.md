# ADK-Rust Target Architecture and Adoption Sequence

Date: 2026-02-14

## Objective

Move `zavora-cli` from a strong CLI baseline to full ADK-Rust capability coverage with controlled, release-based increments.

## Target Architecture

```text
CLI UX Layer
  - ask/chat/workflow/release-plan/sessions
  - runtime slash commands (provider/model/status)
      |
Configuration Layer
  - profile manager (named profiles)
  - provider/model/tooling/retrieval policy
      |
Agent Runtime Layer
  - workflow factory (single/sequential/parallel/loop/graph)
  - session + artifact + memory services
      |
Tooling Layer
  - function tools (schema-bound)
  - MCP toolsets (auth/retry/reconnect controls)
      |
Context Layer
  - retrieval abstraction + optional semantic backend
      |
Quality Layer
  - telemetry + eval harness + guardrail policies + CI quality gates
      |
Execution Modes
  - CLI interactive mode
  - server/A2A mode
```

## Phased Adoption (Sprint-Aligned)

| Phase | Sprint | Adoption Goal | Deliverables |
|---|---|---|---|
| Phase 0 | Sprint 0 | Close architecture uncertainty | Capability matrix, ADR-style architecture doc, risk register |
| Phase 1 | Sprint 1 | Stabilize UX and runtime reliability | Streaming reliability, error taxonomy, session lifecycle commands |
| Phase 2 | Sprint 2 | Introduce profile-first runtime | Profile config, in-session provider/model switching |
| Phase 3 | Sprint 3 | Add context architecture | Retrieval abstraction, optional semantic backend, context budget controls |
| Phase 4 | Sprint 4 | Expand tools/orchestration | MCP manager, tool confirmations, deeper workflow templates |
| Phase 5 | Sprint 5 | Production quality loop | Telemetry baseline, eval harness, guardrails, CI quality thresholds |
| Phase 6 | Sprint 6 | GA runtime hardening | Server/A2A mode, security/perf hardening, migration docs |

## Architectural Constraints

1. Backward CLI compatibility:
- Existing flags and commands must continue working across `v0.x` releases.
- New profile/runtime switching features should be additive first, not breaking.

2. Provider explicitness:
- Provider/model choice must remain observable (`/status` and logs).
- Provider/model compatibility errors must be deterministic and actionable.

3. Feature-gated expansion:
- Retrieval backend and heavy integrations remain optional cargo features.
- Core binary behavior must remain stable when optional features are off.

4. Safety before autonomy:
- Tooling and guardrails must provide explicit policies for risky operations.
- CI must block release promotions when eval/guardrail thresholds regress.

5. Runtime separation:
- CLI interaction flow and server lifecycle should share core runtime modules but keep mode-specific entrypoints.

## Migration Strategy from Current Baseline

1. Keep current CLI contract intact:
- Retain `ask`, `chat`, `workflow`, `release-plan`, `doctor`, `migrate`, `sessions`.

2. Add profile mode incrementally:
- Introduce profile config with defaults mirroring current env-based behavior.
- If profile missing, fallback to existing env + CLI flags.

3. Add runtime switching in chat mode only first:
- Support `/provider`, `/model`, `/status` in interactive mode.
- Validate before switch; preserve session continuity on successful switch.
- On failed switch, keep prior runtime active.

4. Stage retrieval as non-breaking:
- Retrieval disabled by default until configured.
- Context injection bounded by profile policies.

5. Stage quality gates before GA:
- Add eval/guardrail checks in CI as warning mode first.
- Promote to blocking mode after baseline metrics are stable.

## Decision Log

- Decision: profile-driven runtime is the primary configuration path from Sprint 2 onward.
- Decision: retrieval and MCP are additive capabilities behind explicit config/feature gates.
- Decision: `v1.0.0` requires eval + telemetry + guardrail gates and server/A2A smoke coverage.
