# Parity Matrix — zavora-cli vs Q CLI Reference

Status as of v1.1.0-rc (Sprint 11 complete).

## Legend

| Status | Meaning |
|--------|---------|
| ✓ Met | Feature fully implemented and tested |
| ◐ Partial | Feature implemented with known limitations |
| ✗ Not Met | Feature not yet implemented |

## Core Runtime

| Capability | Status | Notes |
|-----------|--------|-------|
| Multi-provider model runtime | ✓ Met | gemini, openai, anthropic, deepseek, groq, ollama |
| Provider/model switching in chat | ✓ Met | /provider, /model, interactive picker |
| Session persistence (SQLite) | ✓ Met | Full CRUD, prune, migration |
| Streaming output | ✓ Met | SSE streaming with guardrail buffering |
| Profile-based configuration | ✓ Met | .zavora/config.toml, CLI overrides, env vars |
| Agent catalogs | ✓ Met | Global/local catalogs, agent selection |

## Tool System

| Capability | Status | Notes |
|-----------|--------|-------|
| Built-in tools (fs_read, fs_write, execute_bash, github_ops) | ✓ Met | Full policy controls |
| MCP tool discovery | ✓ Met | Per-server timeout, auth, diagnostics |
| Tool confirmation policy | ✓ Met | never/mcp-only/always modes |
| Tool aliases and wildcard filtering | ✓ Met | allow/deny with glob patterns |
| Tool timeout and retry | ✓ Met | Configurable per-profile |
| Hook lifecycle (pre/post tool) | ✓ Met | 5 hook points, matcher scoping, pre-tool blocking |

## Chat UX

| Capability | Status | Notes |
|-----------|--------|-------|
| Slash command system | ✓ Met | 13 commands with fuzzy matching |
| Command palette / discovery | ✓ Met | Prefix matching, did-you-mean suggestions |
| Context usage tracking | ✓ Met | Token estimation, budget warnings |
| Manual compaction (/compact) | ✓ Met | Session summarization with event preservation |
| Auto-compaction | ✓ Met | ADK EventsCompactionConfig integration |
| Checkpoint save/list/restore | ✓ Met | In-memory conversation snapshots |
| Tangent mode (enter/exit/tail) | ✓ Met | Branch-and-return with tail variant |
| Todo list persistence | ✓ Met | File-based CRUD in .zavora/todos/ |
| Delegate sub-agent | ◐ Partial | Data model ready, runner wiring deferred |
| First-run onboarding | ✓ Met | Detects missing .zavora/, shows guide |
| Themed prompt with mode indicators | ✓ Met | Tangent, budget level indicators |

## Workflow Modes

| Capability | Status | Notes |
|-----------|--------|-------|
| Single agent | ✓ Met | LlmAgent with tools |
| Sequential agent | ✓ Met | SequentialAgent pipeline |
| Parallel agent | ✓ Met | ParallelAgent + synthesis |
| Loop agent | ✓ Met | LoopAgent + ExitLoopTool |
| Graph agent | ✓ Met | Conditional routing, reusable templates |

## Safety and Observability

| Capability | Status | Notes |
|-----------|--------|-------|
| Guardrail framework (input/output) | ✓ Met | disabled/observe/block/redact modes |
| Telemetry (structured JSONL) | ✓ Met | Events, dashboard report |
| Error redaction | ✓ Met | Sensitive config redacted by default |
| Security hardening | ✓ Met | Path policy, blocked patterns, audit.toml |

## Evaluation and Release

| Capability | Status | Notes |
|-----------|--------|-------|
| Eval harness (retrieval quality) | ✓ Met | Dataset-based, benchmark metrics |
| Parity benchmark suite | ✓ Met | 12 scenarios, scoring rubric, thresholds |
| Release gates (CI) | ✓ Met | make quality-gate, security-check, perf-check |
| Server mode + A2A | ✓ Met | Axum server, healthz, a2a-smoke |

## Summary

- **Met**: 33 capabilities
- **Partial**: 1 (delegate sub-agent — data model ready, runner deferred)
- **Not Met**: 0
- **Parity Score**: 33.5/34 = 98.5%
