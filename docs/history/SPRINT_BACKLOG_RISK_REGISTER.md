# Sprint Backlog and Risk Register

Date: 2026-02-14

## Prioritized Backlog (P0/P1/P2)

| Issue | Title | Priority | Sprint | Milestone |
|---|---|---|---|---|
| [#4](https://github.com/zavora-ai/zavora-cli/issues/4) | Improve chat UX and streaming reliability | P0 | 1 | Sprint 1 - UX Stabilization (`v0.1.1`) |
| [#5](https://github.com/zavora-ai/zavora-cli/issues/5) | Standardize user-facing error taxonomy | P1 | 1 | Sprint 1 - UX Stabilization (`v0.1.1`) |
| [#6](https://github.com/zavora-ai/zavora-cli/issues/6) | Add sessions delete/prune lifecycle commands | P1 | 1 | Sprint 1 - UX Stabilization (`v0.1.1`) |
| [#7](https://github.com/zavora-ai/zavora-cli/issues/7) | Improve command discoverability and docs | P2 | 1 | Sprint 1 - UX Stabilization (`v0.1.1`) |
| [#8](https://github.com/zavora-ai/zavora-cli/issues/8) | Add profile-based config system | P0 | 2 | Sprint 2 - Profiles + Runtime Switching (`v0.2.0`) |
| [#9](https://github.com/zavora-ai/zavora-cli/issues/9) | Add in-session provider switching within same profile | P0 | 2 | Sprint 2 - Profiles + Runtime Switching (`v0.2.0`) |
| [#10](https://github.com/zavora-ai/zavora-cli/issues/10) | Add in-session model switching within same profile | P1 | 2 | Sprint 2 - Profiles + Runtime Switching (`v0.2.0`) |
| [#11](https://github.com/zavora-ai/zavora-cli/issues/11) | Add switching regression and fallback tests | P1 | 2 | Sprint 2 - Profiles + Runtime Switching (`v0.2.0`) |
| [#12](https://github.com/zavora-ai/zavora-cli/issues/12) | Introduce retrieval abstraction layer | P0 | 3 | Sprint 3 - Retrieval Foundation (`v0.2.1`) |
| [#13](https://github.com/zavora-ai/zavora-cli/issues/13) | Add optional semantic search backend (feature-gated) | P1 | 3 | Sprint 3 - Retrieval Foundation (`v0.2.1`) |
| [#14](https://github.com/zavora-ai/zavora-cli/issues/14) | Add context injection policy controls | P1 | 3 | Sprint 3 - Retrieval Foundation (`v0.2.1`) |
| [#15](https://github.com/zavora-ai/zavora-cli/issues/15) | Add retrieval evaluation tests | P1 | 3 | Sprint 3 - Retrieval Foundation (`v0.2.1`) |
| [#16](https://github.com/zavora-ai/zavora-cli/issues/16) | Integrate MCP toolset manager | P0 | 4 | Sprint 4 - Tools + Orchestration (`v0.3.0`) |
| [#17](https://github.com/zavora-ai/zavora-cli/issues/17) | Implement tool confirmation and safety controls | P0 | 4 | Sprint 4 - Tools + Orchestration (`v0.3.0`) |
| [#18](https://github.com/zavora-ai/zavora-cli/issues/18) | Expand workflow templates and graph orchestration | P1 | 4 | Sprint 4 - Tools + Orchestration (`v0.3.0`) |
| [#19](https://github.com/zavora-ai/zavora-cli/issues/19) | Add tool execution reliability controls | P1 | 4 | Sprint 4 - Tools + Orchestration (`v0.3.0`) |
| [#20](https://github.com/zavora-ai/zavora-cli/issues/20) | Add telemetry baseline and reporting | P1 | 5 | Sprint 5 - Eval + Observability + Guardrails (`v0.3.1`) |
| [#21](https://github.com/zavora-ai/zavora-cli/issues/21) | Add evaluation harness and benchmark suite | P0 | 5 | Sprint 5 - Eval + Observability + Guardrails (`v0.3.1`) |
| [#22](https://github.com/zavora-ai/zavora-cli/issues/22) | Add configurable guardrail policy framework | P0 | 5 | Sprint 5 - Eval + Observability + Guardrails (`v0.3.1`) |
| [#23](https://github.com/zavora-ai/zavora-cli/issues/23) | Integrate quality gates into CI | P1 | 5 | Sprint 5 - Eval + Observability + Guardrails (`v0.3.1`) |
| [#24](https://github.com/zavora-ai/zavora-cli/issues/24) | Add server and A2A runtime mode | P0 | 6 | Sprint 6 - GA Hardening (`v1.0.0`) |
| [#25](https://github.com/zavora-ai/zavora-cli/issues/25) | Complete security hardening pass | P0 | 6 | Sprint 6 - GA Hardening (`v1.0.0`) |
| [#26](https://github.com/zavora-ai/zavora-cli/issues/26) | Execute performance and reliability tests | P1 | 6 | Sprint 6 - GA Hardening (`v1.0.0`) |
| [#27](https://github.com/zavora-ai/zavora-cli/issues/27) | Publish GA docs and migration guide | P1 | 6 | Sprint 6 - GA Hardening (`v1.0.0`) |

## Sprint Execution Sequence

1. Sprint 1 focus: close all P0/P1 items (`#4`, `#5`, `#6`) before P2 (`#7`).
2. Sprint 2 focus: implement runtime profile/switching contract first (`#8`, `#9`), then compatibility/test hardening (`#10`, `#11`).
3. Sprint 3-4 focus: establish retrieval abstraction before semantic backend, then introduce MCP/tool safety controls before deeper orchestration.
4. Sprint 5-6 focus: enforce quality gates before GA hardening sign-off.

## Risk Register

| Risk | Probability | Impact | Owner | Mitigation | Trigger / Monitoring |
|---|---|---|---|---|---|
| Runtime switching causes session corruption | Medium | High | @jkmaina | Add switch rollback path and regression tests (`#11`) before enabling by default | Any failed switch leaves invalid provider/model state |
| Provider capability mismatch across models | Medium | High | @jkmaina | Add compatibility matrix checks in `/provider` and `/model` commands (`#9`, `#10`) | User selects unsupported model for provider |
| Retrieval integration increases token usage unexpectedly | Medium | Medium | @jkmaina | Enforce prompt/context budget policy (`#14`) and retrieval evals (`#15`) | Prompt length warnings or latency spikes |
| MCP integration introduces unstable tool latency | Medium | High | @jkmaina | Standardize timeout/retry and failure handling (`#19`) with safe defaults | Tool calls exceed timeout or retries saturate |
| No measurable quality baseline before release | High | High | @jkmaina | Prioritize eval harness + CI gate rollout (`#21`, `#23`) in Sprint 5 | Release candidate passes without quality signal |
| Guardrails block legitimate outputs or miss unsafe outputs | Medium | High | @jkmaina | Define policy severity levels and test blocking/redaction paths (`#22`) | Spike in false positives/false negatives |
| Security/performance debt slips into GA | Medium | High | @jkmaina | Gate `v1.0.0` on security + performance milestones (`#25`, `#26`) | Unresolved critical findings near GA cutoff |

## Sprint 0 Completion Mapping

- Issue [#1](https://github.com/zavora-ai/zavora-cli/issues/1): delivered by `/Users/jameskaranja/Developer/projects/zavora-cli/docs/ADK_CAPABILITY_MATRIX.md`
- Issue [#2](https://github.com/zavora-ai/zavora-cli/issues/2): delivered by `/Users/jameskaranja/Developer/projects/zavora-cli/docs/ADK_TARGET_ARCHITECTURE.md`
- Issue [#3](https://github.com/zavora-ai/zavora-cli/issues/3): delivered by `/Users/jameskaranja/Developer/projects/zavora-cli/docs/SPRINT_BACKLOG_RISK_REGISTER.md`
