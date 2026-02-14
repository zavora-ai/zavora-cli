# ADK-Rust Capability Matrix

Date: 2026-02-14  
Scope: `zavora-cli` baseline as implemented in `/Users/jameskaranja/Developer/projects/zavora-cli/src/main.rs`

## Coverage Matrix (Current vs Target)

| Capability | Target State | Current State | Evidence | Gap | Impact | Effort | Dependencies / Blockers |
|---|---|---|---|---|---|---|---|
| Core single-agent execution | `LlmAgent` for direct prompt flows | Implemented | `ask`, `chat` commands; `build_single_agent` | None | Medium | Low | None |
| Sequential workflow | Ordered staged flow with state handoff | Implemented | `WorkflowMode::Sequential`; `build_sequential_agent` | None | Medium | Low | None |
| Parallel workflow | Independent analyzers + synthesis | Implemented | `WorkflowMode::Parallel`; `build_parallel_agent` | None | Medium | Low | None |
| Loop workflow | Iterative loop with explicit termination | Implemented | `WorkflowMode::Loop`; `LoopAgent` + `ExitLoopTool` | None | Medium | Low | None |
| Graph orchestration | Routing/checkpoints/interrupts/state reducers | Missing | No `adk-graph` usage in crate | Graph-based orchestration path absent | High | High | Add `adk-graph` dependency, design state schema |
| Provider support | Multi-provider model runtime | Implemented (flag/env based) | `Provider` enum + `resolve_model` | No profile-scoped defaults | High | Medium | Config format + validation layer |
| In-session provider/model switching | Switch provider/model without restart | Missing | No `/provider`/`/model` runtime command | Session-time switching unavailable | High | Medium | Session-safe agent rebuild logic |
| Profile-based runtime config | Named profiles with provider/model/tooling policies | Missing | Only CLI args/env vars today | No portable profile config | High | Medium | TOML/YAML schema + migration path |
| Session persistence | Memory + SQLite backends | Implemented | `SessionBackend::{Memory,Sqlite}` + migrate command | No delete/prune lifecycle ops | Medium | Low | Add command handlers + safety confirmations |
| Retrieval abstraction | Pluggable retrieval interfaces and adapters | Missing | No retrieval trait/service | No grounding mechanism | High | Medium | Trait + adapter boundary + feature flags |
| Optional semantic search backend | Feature-gated semantic store | Missing | No semantic backend integration | No advanced retrieval backend | Medium | Medium | Backend choice + cargo feature wiring |
| Function tool contracts | Explicit schemas and stable output shapes | Partial | `FunctionTool` used; args not schema-validated | Tool contract safety incomplete | Medium | Medium | Schema validation wrapper |
| MCP toolsets | External tool registry and invocation | Missing | No MCP client/toolset manager | Cannot consume MCP tools | High | Medium | MCP auth/retry config model |
| ADK UI protocol outputs | UI render tool outputs with protocol compatibility | Missing | No UI output protocol tooling | No UI-capable outputs | Medium | Medium | Protocol contract tests |
| Observability | Correlated telemetry spans across model/tool/session | Partial | Basic `tracing` logs only | No correlation/dashboard baseline | High | Medium | Event model + dashboard sink |
| Evaluation harness | Dataset + rubric + threshold gates | Missing | Only unit tests with `MockLlm` | No measurable quality loop | High | Medium | Eval dataset/versioning design |
| Guardrails | PII/content/schema enforcement with policy outcomes | Missing | No guardrail policy framework | Unsafe output path risk | High | Medium | Policy engine + block/redact flow |
| CI quality gates | Eval/guardrail thresholds in CI | Missing | `make ci` exists; no eval gates | Regressions can pass CI | High | Medium | Eval command + threshold config |
| Server / A2A runtime mode | Service mode alongside CLI | Missing | CLI-only command surface | No multi-client runtime mode | High | High | Runtime mode split + smoke tests |

## Ranked Gaps (Impact vs Effort)

Rank key: prioritize highest impact with feasible effort first.

| Rank | Capability Gap | Impact | Effort | Why This Rank |
|---|---|---|---|---|
| 1 | Profile-based config + runtime switching | High | Medium | Unblocks Sprint 2 core objective and operator ergonomics |
| 2 | Retrieval abstraction | High | Medium | Enables context-aware behavior and future backends |
| 3 | MCP toolsets + tool safety contracts | High | Medium | Expands real agent utility and operational safety |
| 4 | Eval + telemetry + guardrails | High | Medium | Required for production confidence and release gating |
| 5 | Session lifecycle delete/prune | Medium | Low | Quick operational win; needed for state hygiene |
| 6 | Semantic backend (feature-gated) | Medium | Medium | Valuable extension once abstraction exists |
| 7 | Server/A2A mode | High | High | Large architectural expansion best deferred to GA sprint |
| 8 | Graph orchestration | High | High | Powerful but complexity-heavy; align with advanced workflows |

## Dependency and Blocker Notes

- Runtime switching depends on a profile layer that can validate provider/model compatibility before re-binding an active runner.
- Retrieval backend work should not start before retrieval interfaces are stable and prompt-budget policy is defined.
- MCP integration depends on auth model and retry/timeout defaults to avoid runtime hangs.
- Eval/guardrail CI gates depend on deterministic eval datasets and explicit promotion thresholds.
- Server/A2A rollout depends on clean separation between CLI interactive flow and long-lived service lifecycle.
