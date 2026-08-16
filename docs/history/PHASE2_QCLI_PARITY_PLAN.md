# Phase 2: Q-Style Parity And UX Excellence

Date: February 15, 2026

Project board: [Zavora CLI Phase 2 - Q Parity + UX](https://github.com/orgs/zavora-ai/projects/5)

## Why Phase 2

`zavora-cli` has strong ADK-Rust foundations (workflow topology, telemetry/eval/guardrails, server + A2A), but coding UX and tool ergonomics still lag mature coding-first CLIs.

Phase 2 goal: match and exceed Q-style developer UX while preserving Zavora security posture and release rigor.

## UI/UX Assessment Highlights

Reference CLI strengths to match:

- Rich slash-command surface (`/model`, `/tools`, `/mcp`, `/usage`, `/compact`, etc.)
- Interactive model picker UX
- Strong built-in coding tools (`fs_read`, `fs_write`, `execute_bash`) with policy controls
- Mature MCP ergonomics and diagnostics
- Context visibility and control (usage indicators + auto compaction)
- Conversation branching and task continuity (checkpoint/tangent/todo/delegate)
- Agent files, hooks, and workspace-aware behavior

Current Zavora strengths:

- ADK workflow depth (`single`, `sequential`, `parallel`, `loop`, `graph`)
- Provider/runtime switching and compatibility checks
- Guardrails, telemetry, eval, security gate, perf gate
- Server mode and A2A runtime flow

Primary gaps:

- Coding tool depth and safety controls at parity level
- Slash-command breadth and interaction polish
- Context intelligence UX (compaction, usage, branching)
- Agent file/hook system maturity

## Recommended Architecture (Phase 2)

1. UX Shell Layer
   - Slash command dispatcher
   - Interactive pickers and discoverability surfaces
2. Tooling Layer
   - Native coding tools with strict policy contracts
   - MCP ecosystem with diagnostics, aliasing, and permission patterns
3. Agent Layer
   - Local/global agent files, resources, hooks, model selection
4. Context Intelligence Layer
   - Usage indicators, manual/auto compaction, branching/todo/delegate
5. Quality Layer
   - Benchmarks, parity scorecards, release gates and rollback evidence

## Sprint Plan (S7-S11)

## Sprint 7 - Security + UX Foundation (`v1.1.0-alpha.1`)

- [#28](https://github.com/zavora-ai/zavora-cli/issues/28) Redact sensitive runtime config from CLI output and logs
- [#29](https://github.com/zavora-ai/zavora-cli/issues/29) Introduce slash-command framework and discoverability UX in chat
- [#30](https://github.com/zavora-ai/zavora-cli/issues/30) Add interactive model picker and model metadata UX
- [#31](https://github.com/zavora-ai/zavora-cli/issues/31) Publish working style contract and execution playbook

## Sprint 8 - Coding Core Parity (`v1.1.0-alpha.2`)

- [#32](https://github.com/zavora-ai/zavora-cli/issues/32) Add fs_read tool with path policy controls
- [#33](https://github.com/zavora-ai/zavora-cli/issues/33) Add fs_write/edit tool with safe patch workflow
- [#34](https://github.com/zavora-ai/zavora-cli/issues/34) Add execute_bash tool with allow deny policy engine
- [#35](https://github.com/zavora-ai/zavora-cli/issues/35) Add GitHub workflow toolkit for issues PRs and project operations

## Sprint 9 - Agent + MCP Maturity (`v1.1.0-beta.1`)

- [#36](https://github.com/zavora-ai/zavora-cli/issues/36) Implement agent file format with local global precedence
- [#37](https://github.com/zavora-ai/zavora-cli/issues/37) Add tool aliases and wildcard permission patterns
- [#38](https://github.com/zavora-ai/zavora-cli/issues/38) Implement hook lifecycle events for agent and tool execution
- [#39](https://github.com/zavora-ai/zavora-cli/issues/39) Expand MCP diagnostics and resilience UX

## Sprint 10 - Context Intelligence (`v1.1.0-beta.2`)

- [#40](https://github.com/zavora-ai/zavora-cli/issues/40) Add context usage indicator and budget warning UX
- [#41](https://github.com/zavora-ai/zavora-cli/issues/41) Implement manual and automatic context compaction
- [#42](https://github.com/zavora-ai/zavora-cli/issues/42) Add checkpoint and tangent conversation branching
- [#43](https://github.com/zavora-ai/zavora-cli/issues/43) Add todo list and delegate sub-agent experiments

## Sprint 11 - UX Polish + Parity Validation (`v1.1.0-rc.1`)

- [#44](https://github.com/zavora-ai/zavora-cli/issues/44) Deliver cohesive UI theme command palette and onboarding UX
- [#45](https://github.com/zavora-ai/zavora-cli/issues/45) Build coding parity benchmark suite and scorecard
- [#46](https://github.com/zavora-ai/zavora-cli/issues/46) Close top parity gaps and publish differentiation roadmap
- [#47](https://github.com/zavora-ai/zavora-cli/issues/47) Prepare v1.1.0 RC release plan and operational sign-off

## Success Criteria

- Coding workflows (project creation, file operations, git/github flows) are first-class.
- MCP and tool permissions are governable and observable.
- Long-running sessions remain usable due to context controls.
- UX is intentionally designed, not purely functional.
- Phase 2 release candidate ships with benchmark evidence and rollback readiness.
