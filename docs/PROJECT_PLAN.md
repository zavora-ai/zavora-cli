# Zavora CLI Project Plan

This plan combines the initial release-train roadmap with an end-to-end ADK-Rust capability adoption path.

## Planning Assumptions

- Sprint length: 2 weeks
- Release model: incremental SemVer tags
- Current baseline: `v0.1.0` shipped
- Goal: move from a strong CLI baseline to full ADK-Rust capability coverage with production readiness

## Milestone Roadmap

| Sprint | Theme | Primary Goal | Release Target |
|---|---|---|---|
| Sprint 0 | Capability Audit | Map ADK-Rust end-to-end capability gaps and define adoption architecture | Internal planning milestone |
| Sprint 1 | UX Stabilization | Harden CLI UX, reliability, and day-to-day developer flow | `v0.1.1` |
| Sprint 2 | Profiles + Runtime Switching | Add profiles and in-session provider/model switching within same profile | `v0.2.0` |
| Sprint 3 | Retrieval Foundation | Add retrieval abstraction and optional semantic search backend | `v0.2.1` |
| Sprint 4 | Tools + Orchestration | Expand MCP/tooling and workflow orchestration depth | `v0.3.0` |
| Sprint 5 | Eval + Observability + Guardrails | Add measurable quality loop and safety controls | `v0.3.1` |
| Sprint 6 | GA Hardening | Production hardening, server mode, and GA release quality | `v1.0.0` |

## ADK-Rust Capability Coverage Targets

- Agents/workflows: LLM + Sequential + Parallel + Loop + Graph orchestration
- Tools: Function tools + MCP toolsets + confirmation controls
- Runtime: sessions/artifacts/memory + server/A2A mode
- Quality: eval harness + telemetry + guardrails
- Developer experience: profile-driven config + sprint/release governance

## Definition of Done (Per Sprint)

- Scope complete and acceptance criteria met
- `make release-check` passes
- Docs and changelog updated
- Sprint issues closed and milestone reviewed

## Risk Controls

- Keep optional heavy features behind flags
- Require deterministic test coverage for new orchestration modes
- Add explicit rollout/rollback notes for release-affecting changes
