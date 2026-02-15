# Project Optimisation Plan

This document captures the full end-to-end review findings and the concrete changes applied to address them.

## Architecture: Module Split

**Problem:** The entire project (8,911 LOC) lives in a single `src/main.rs`, making it unmaintainable, untestable at module boundaries, and hostile to contributors.

**Solution:** Split into `src/lib.rs` + focused module files:

| Module | Responsibility |
|---|---|
| `main.rs` | Entry point only (`main`, `run_cli`) |
| `lib.rs` | Module declarations and re-exports |
| `cli.rs` | `Cli`, `Commands`, all subcommand enums, `ValueEnum` types |
| `config.rs` | `RuntimeConfig`, `ProfilesFile`, `ProfileConfig`, profile/agent resolution |
| `error.rs` | `ErrorCategory`, `categorize_error`, `format_cli_error`, redaction |
| `provider.rs` | `detect_provider`, `resolve_model`, `validate_model_for_provider` |
| `session.rs` | Session service building, session CRUD commands, SQLite helpers |
| `runner.rs` | `build_runner` variants, `resolve_runtime_tools`, tool confirmation |
| `tools/mod.rs` | `build_builtin_tools`, shared tool types |
| `tools/fs_read.rs` | `FsReadRequest`, workspace path policy, `fs_read_tool_response` |
| `tools/fs_write.rs` | `FsWriteRequest`, `fs_write_tool_response` |
| `tools/execute_bash.rs` | `ExecuteBashRequest`, policy evaluation, `execute_bash_tool_response` |
| `tools/github_ops.rs` | GitHub CLI operations, auth preflight |
| `workflow.rs` | Workflow agents (sequential, parallel, loop, graph), route classifier |
| `retrieval.rs` | `RetrievalService` trait, local/semantic backends, prompt augmentation |
| `mcp.rs` | MCP server selection, auth resolution, tool discovery |
| `server.rs` | Axum server, health/ask/A2A handlers, runner cache |
| `chat.rs` | Interactive chat mode, slash commands, model picker |
| `streaming.rs` | `AuthorTextTracker`, text deduplication, stream suffix |
| `telemetry.rs` | `TelemetrySink`, `TelemetrySummary`, report command |
| `guardrail.rs` | Guardrail modes, term matching, redaction, `apply_guardrail` |
| `eval.rs` | Eval dataset, harness, benchmark metrics, report writing |
| `doctor.rs` | `run_doctor`, `run_migrate` |
| `profiles.rs` | `run_profiles_list`, `run_profiles_show` |
| `agents.rs` | Agent catalog loading, selection, list/show/select commands |

## Security Fixes

### Server Authentication

**Problem:** `/v1/ask` accepts arbitrary prompts with no auth. Caller can override `user_id`/`session_id` to hijack sessions.

**Fix:** Add optional bearer token auth via `ZAVORA_SERVER_AUTH_TOKEN` env var. When set, all server endpoints (except `/healthz`) require `Authorization: Bearer <token>`. Remove caller-controlled `user_id` override from `ServerAskRequest`.

### execute_bash Command Chaining

**Problem:** Read-only detection is prefix-based and trivially bypassed with `ls; rm -rf /`.

**Fix:** Add command chaining detection that rejects commands containing `;`, `&&`, `||`, `|`, backticks, or `$()` when the command starts with a read-only prefix. Chained commands require `approved=true`.

### Prompt Size Limits

**Problem:** No limits on prompt length. Large prompts can cause OOM or excessive API billing.

**Fix:** Add `max_prompt_chars` config field (default 32,000) enforced in `ask`, `chat`, `workflow`, and `/v1/ask`.

## Reliability Fixes

### Graceful Server Shutdown

**Problem:** `axum::serve` has no shutdown signal handler. Ctrl+C hard-kills without draining.

**Fix:** Add `tokio::signal` handler for SIGINT/SIGTERM with graceful shutdown via `axum::serve(...).with_graceful_shutdown(...)`.

### Runner Cache Eviction

**Problem:** Server runner cache grows unboundedly per `user_id::session_id` pair.

**Fix:** Add LRU-style eviction with configurable `server_runner_cache_max` (default 64). Evict oldest entry when limit is reached.

### Telemetry Write Safety

**Problem:** `TelemetrySink::append_event_line` opens/writes/closes on every event with no synchronization. Concurrent server requests can corrupt JSONL.

**Fix:** Wrap the file handle in `Arc<std::sync::Mutex<...>>` for thread-safe appends. The mutex is held only for the duration of a single line write.

## Build & CI Fixes

### Cargo.toml Metadata

Add `description`, `license`, `repository`, `authors`, `rust-version` fields.

### .gitignore Expansion

Add `.DS_Store`, `*.swp`, `*.swo`, `.idea/`, `.vscode/`, `*.bak`.

### CI Security Check

**Problem:** `security-check` only runs in release workflow, not on PRs.

**Fix:** Add `security-check` to the `ci` Makefile target.

## Findings Deferred

The following items are documented but deferred to future work:

- **Blocking I/O in async contexts** — Requires `tokio::task::spawn_blocking` wrappers around all sync fs/process calls. Deferred to avoid destabilizing the module split.
- **Semantic retrieval scoring** — Jaro-Winkler is inappropriate for long texts. Requires BM25 or embedding-based approach.
- **Cross-platform CI** — macOS/Windows runners.
- **Cross-platform release builds** — macOS/ARM artifacts.
- **Rustdoc comments** — Zero doc comments currently. Should be added per-module after split.
- **Server endpoint tests** — Integration tests for health/ask/A2A handlers.
- **Concurrent telemetry/cache tests** — Stress tests for thread safety.
