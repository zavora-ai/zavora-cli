# Requirements: ADK Crate Adoption for Zavora-CLI

## Context

Zavora-CLI v1.1.5 currently uses only 3 of 30+ ADK-Rust crates (`adk-rust`, `adk-session`, `adk-tool`). The ADK monorepo provides production-ready implementations for skills, memory, telemetry, guardrails, plugins, browser automation, code execution, and RAG — all with builder APIs that integrate directly into the existing `Runner::builder()` chain. Adopting these replaces hand-rolled implementations with battle-tested, maintained code.

### Current State

| Capability | Current Implementation | Lines | Quality |
|-----------|----------------------|-------|---------|
| Skills/context injection | None | 0 | Missing |
| Memory | Hand-rolled `memory.json` + `MemoryAgentTool` | ~200 | Basic, no semantic search |
| Telemetry | Hand-rolled JSONL to `.zavora/telemetry/events.jsonl` | ~200 | No tracing, no OTLP |
| Guardrails | Hand-rolled regex in `guardrail.rs` | ~130 | No PII detection, basic keyword matching |
| Plugins/hooks | None (ConfirmingTool wrapper only) | 0 | Missing |
| Browser automation | None | 0 | Missing |
| Code execution | Raw `execute_bash` | ~400 | No sandboxing, no WASM |
| RAG/retrieval | Basic `retrieval.rs` with local file search | ~200 | No embeddings, no vector store |
| File history/undo | None | 0 | Missing |

### ADK Crates Available

| Crate | Integration Point | API |
|-------|------------------|-----|
| `adk-skill` | `Runner::with_auto_skills(root, config)` | One-liner on builder |
| `adk-memory` | `Runner::builder().memory_service(sqlite_memory)` | One-liner on builder |
| `adk-telemetry` | `init_telemetry("zavora-cli")` | One function call |
| `adk-guardrail` | `PiiRedactor::new()`, `ContentFilter::harmful_content()` | Drop-in replacements |
| `adk-plugin` | `Runner::builder().plugin_manager(pm)` | One-liner on builder |
| `adk-browser` | `BrowserSession::new()` → 15+ tool structs | Register as tools |
| `adk-code` + `adk-sandbox` | `CodeTool::new(executor)`, `SandboxTool::new(backend)` | Register as tools |
| `adk-rag` | `RagPipeline::builder().embedding_provider().vector_store().build()` | Replace retrieval.rs |

---

## REQ-1: Skill System (`adk-skill`)

### REQ-1.1
Zavora-cli MUST support `.zavora/skills/` and `~/.zavora/skills/` directories containing Markdown skill files per the agentskills.io specification.

### REQ-1.2
Skills MUST be auto-discovered and injected into agent context via `Runner::with_auto_skills()`.

### REQ-1.3
A `zavora skills list` CLI command MUST display discovered skills with their metadata.

### REQ-1.4
Skills MUST be injectable per-agent via agent catalog config: `skill_paths = ["./skills/rust-expert.md"]`.

---

## REQ-2: Semantic Memory (`adk-memory`)

### REQ-2.1
The hand-rolled `memory.json` system MUST be replaced with `adk-memory::SqliteMemoryService` backed by `.zavora/memory.db`.

### REQ-2.2
The existing `/memory recall|remember|forget` chat commands MUST continue to work with the new backend.

### REQ-2.3
Memory search MUST support semantic similarity (not just keyword matching) when an embedding provider is configured.

### REQ-2.4
The `MemoryAgentTool` MUST be updated to delegate to the ADK memory service instead of the hand-rolled JSON store.

### REQ-2.5
Existing `memory.json` data MUST be migrated to SQLite on first run.

---

## REQ-3: OpenTelemetry (`adk-telemetry`)

### REQ-3.1
The hand-rolled JSONL telemetry MUST be replaced with `adk-telemetry` OpenTelemetry integration.

### REQ-3.2
Agent runs, model calls, and tool executions MUST produce OpenTelemetry spans with proper parent-child relationships.

### REQ-3.3
OTLP export MUST be configurable via `OTEL_EXPORTER_OTLP_ENDPOINT` environment variable.

### REQ-3.4
The existing `TelemetrySink` API MUST be preserved as a thin wrapper for backward compatibility.

### REQ-3.5
A `zavora telemetry` command MUST continue to work, reading from the OpenTelemetry exporter or falling back to JSONL.

---

## REQ-4: Guardrails (`adk-guardrail`)

### REQ-4.1
The hand-rolled regex guardrails MUST be replaced with `adk-guardrail` components: `PiiRedactor` for PII detection/redaction and `ContentFilter` for content policy.

### REQ-4.2
PII redaction MUST detect: email, phone, SSN, credit card, IP address.

### REQ-4.3
Content filtering MUST support: harmful content blocking, keyword blocklists, max output length.

### REQ-4.4
The existing `--guardrail-input-mode` and `--guardrail-output-mode` CLI flags MUST continue to work.

### REQ-4.5
Guardrail configuration MUST be extensible via profile config:
```toml
[profiles.default.guardrails]
pii_redaction = true
content_filter = "harmful"
max_output_length = 50000
blocked_keywords = ["password", "secret"]
```

---

## REQ-5: Plugin System (`adk-plugin`)

### REQ-5.1
The ADK plugin system MUST be wired into the runner to enable lifecycle hooks.

### REQ-5.2
A **file history plugin** MUST be implemented using `before_tool` hooks that snapshots file contents before any write tool (`fs_write`, `file_edit`) executes.

### REQ-5.3
An `/undo` chat command MUST restore the last file modified by a write tool to its pre-edit state.

### REQ-5.4
File history MUST be stored in `.zavora/file_history/` with one snapshot per file per tool call.

### REQ-5.5
A maximum of 20 snapshots per file MUST be retained (oldest pruned).

### REQ-5.6
Plugins MUST be configurable via profile config:
```toml
[profiles.default.plugins]
file_history = true
```

---

## REQ-6: Browser Automation (`adk-browser`)

### REQ-6.1
Browser automation tools MUST be available as an optional feature flag (`browser`).

### REQ-6.2
The following `adk-browser` tools MUST be registered: `NavigateTool`, `ClickTool`, `TypeTextTool`, `ExtractTool`, `ScreenshotTool`, `WaitTool`, `EvaluateTool`, `CookiesTool`.

### REQ-6.3
A `BrowserSession` MUST be lazily initialized on first browser tool use and reused for the session.

### REQ-6.4
Browser tools MUST require confirmation (not auto-approved) since they have side effects.

### REQ-6.5
The browser session MUST be closed on chat exit.

---

## REQ-7: Code Execution Sandbox (`adk-code` + `adk-sandbox`)

### REQ-7.1
A `code_execute` tool MUST be available as an optional feature flag (`sandbox`).

### REQ-7.2
The tool MUST support Rust code execution via `adk-code::RustExecutor` with `adk-sandbox::ProcessBackend`.

### REQ-7.3
Code execution MUST be sandboxed — no filesystem access outside a temp directory, no network access.

### REQ-7.4
The tool MUST return: stdout, stderr, exit code, execution time.

### REQ-7.5
The tool MUST require confirmation since it executes arbitrary code.

### REQ-7.6
When the `wasm` feature of `adk-sandbox` is enabled, WASM sandboxing MUST be preferred over process sandboxing.

---

## REQ-8: RAG Pipeline (`adk-rag`)

### REQ-8.1
The hand-rolled `retrieval.rs` MUST be replaced with `adk-rag::RagPipeline`.

### REQ-8.2
Document ingestion MUST support: Markdown, plain text, and PDF files.

### REQ-8.3
The default vector store MUST be in-memory (`adk-rag::InMemoryVectorStore`) with optional SQLite persistence.

### REQ-8.4
Embedding MUST use the configured LLM provider's embedding API when available, falling back to local TF-IDF.

### REQ-8.5
The existing `--retrieval-backend` and `--retrieval-doc-path` CLI flags MUST continue to work.

### REQ-8.6
A `zavora rag ingest <path>` command MUST be added for manual document ingestion.

### REQ-8.7
RAG results MUST be injected into the prompt via the existing retrieval augmentation flow.

---

## Priority and Phasing

### Phase 1 — Drop-in replacements (one-liner integrations)
- REQ-1: Skill System (one line on Runner builder)
- REQ-2: Semantic Memory (replace JSON with SQLite)
- REQ-3: OpenTelemetry (replace JSONL with OTLP)

### Phase 2 — Enhanced safety
- REQ-4: Guardrails (replace regex with PII + content filter)
- REQ-5: Plugin System + File History/Undo

### Phase 3 — New capabilities
- REQ-6: Browser Automation (feature-gated)
- REQ-7: Code Sandbox (feature-gated)
- REQ-8: RAG Pipeline (replace retrieval.rs)
