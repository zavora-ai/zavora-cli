# GitHub Milestone and Issue Breakdown

This document defines the milestone/issue structure used for execution.

## Sprint 0 - ADK Capability Audit

1. **[S0] Build ADK capability matrix and gap assessment**
Acceptance Criteria:
- Document current vs target ADK-Rust capability coverage in `docs/ADK_CAPABILITY_MATRIX.md`
- Rank gaps by impact/effort
- Identify dependencies and blockers per capability

2. **[S0] Define target architecture and adoption sequence**
Acceptance Criteria:
- Publish architecture decision doc with phased adoption order
- Include constraints for provider/tooling/runtime choices
- Include migration strategy for current CLI behavior

3. **[S0] Create prioritized backlog and risk register**
Acceptance Criteria:
- Publish sprint-ready backlog with P0/P1/P2 tags
- Add top risks with mitigation owners
- Link backlog items to Sprint 1-6 milestones

## Sprint 1 - UX Stabilization (`v0.1.1`)

1. **[S1] Improve chat UX and response streaming reliability**
Acceptance Criteria:
- Streaming output is stable for supported providers
- No duplicate/empty response regressions in CLI chat mode
- Add regression tests for streaming/finalization behavior

2. **[S1] Standardize user-facing error taxonomy**
Acceptance Criteria:
- Common failure classes map to clear actionable messages
- Provider/session/tooling errors are distinguishable
- Error paths covered by tests for core commands

3. **[S1] Complete session lifecycle command set**
Acceptance Criteria:
- Add `sessions delete` and `sessions prune` commands
- Document retention behavior for memory/sqlite backends
- Add tests for destructive command safety checks

4. **[S1] Improve command discoverability and docs**
Acceptance Criteria:
- Help text covers main workflows and switching behavior
- README quickstart aligned with current commands
- Release checklist references updated docs

## Sprint 2 - Profiles + Runtime Switching (`v0.2.0`)

1. **[S2] Add profile-based config system**
Acceptance Criteria:
- Config file supports named profiles with provider/model defaults
- CLI can select profile and show active profile state
- Invalid profile configuration fails with actionable diagnostics

2. **[S2] Add in-session provider switching within same profile**
Acceptance Criteria:
- Add runtime switch command (e.g. `/provider <name>`)
- Provider capability/env validation before switch
- Session continuity is preserved after switch

3. **[S2] Add in-session model switching within same profile**
Acceptance Criteria:
- Add runtime switch command (e.g. `/model <id>`)
- Model/provider compatibility checks enforced
- Add `/status` command showing active provider/model/profile

4. **[S2] Add switching regression + fallback tests**
Acceptance Criteria:
- Tests cover successful/failed provider and model switches
- Fallback behavior documented when switch fails
- No crash or silent state corruption across switches

## Sprint 3 - Retrieval Foundation (`v0.2.1`)

1. **[S3] Introduce retrieval abstraction layer**
Acceptance Criteria:
- Retrieval trait/interface added with pluggable adapters
- Existing flows can run with retrieval disabled
- Integration points documented

2. **[S3] Add optional semantic search backend (feature-gated)**
Acceptance Criteria:
- Backend integrated behind cargo feature flag
- Build and run unaffected when feature disabled
- Basic retrieval smoke tests pass when enabled

3. **[S3] Add context injection policy controls**
Acceptance Criteria:
- Control max injected context and ranking limits
- Include profile-level retrieval policy options
- Ensure prompt budgets are respected

4. **[S3] Add retrieval evaluation tests**
Acceptance Criteria:
- Add deterministic retrieval correctness checks
- Track fallback behavior when no results are found
- Include failure-path tests for backend unavailability

## Sprint 4 - Tools and Orchestration (`v0.3.0`)

1. **[S4] Integrate MCP toolset manager**
Acceptance Criteria:
- MCP toolset can be registered/configured per profile
- Tool discovery and invocation flow documented
- Failure handling for unreachable MCP services implemented

2. **[S4] Implement tool confirmation and safety controls**
Acceptance Criteria:
- Confirmation policies configurable per tool/profile
- Safe defaults for high-risk tool operations
- Tests verify confirmation enforcement

3. **[S4] Expand workflow orchestration templates**
Acceptance Criteria:
- Add reusable workflow templates for common tasks
- Include graph/orchestration path for complex flows
- Add tests for orchestration branch behavior

4. **[S4] Add tool execution reliability controls**
Acceptance Criteria:
- Timeouts/retries standardized for tool execution
- Structured error reporting for tool failures
- Telemetry hooks added for tool lifecycle events

## Sprint 5 - Eval, Observability, Guardrails (`v0.3.1`)

1. **[S5] Add telemetry baseline and dashboards**
Acceptance Criteria:
- Structured telemetry emitted for key execution paths
- Define minimal dashboard/report outputs
- Telemetry can be toggled by environment/profile

2. **[S5] Add evaluation harness and benchmark suite**
Acceptance Criteria:
- Curated eval dataset/versioning added
- Automated eval command integrated into workflow
- Baseline quality metrics published per release

3. **[S5] Add guardrail policy framework**
Acceptance Criteria:
- Input/output guardrail policies configurable
- Guardrail outcomes are observable and testable
- Blocking/redaction paths covered by tests

4. **[S5] Integrate quality gates into CI**
Acceptance Criteria:
- CI includes eval/guardrail checks with thresholds
- Clear failure messages for quality regressions
- Release checklist updated with quality gate expectations

## Sprint 6 - GA Hardening (`v1.0.0`)

1. **[S6] Add server/A2A runtime mode**
Acceptance Criteria:
- Server mode runs with documented startup configuration
- A2A interaction path validated with smoke tests
- CLI and server modes can coexist cleanly

2. **[S6] Perform security hardening pass**
Acceptance Criteria:
- Secret handling, dependency posture, and threat notes reviewed
- Add security checks to release process
- Document key operational security controls

3. **[S6] Execute performance and reliability testing**
Acceptance Criteria:
- Define target latency/resource metrics
- Run load/stress tests and publish summary
- Address top bottlenecks before GA

4. **[S6] Publish GA documentation and migration guide**
Acceptance Criteria:
- End-user and operator docs complete
- Migration notes from pre-1.0 versions published
- Final GA checklist signed off
