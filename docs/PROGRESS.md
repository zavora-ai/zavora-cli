# Project Optimisation — Implementation Progress

**Branch:** `feat/project-optimisation`
**Started:** 2026-02-15
**Base commit on main:** `da2770d` (chore: add async_trait import and tool_aliases field to McpServerConfig)

---

## Status Summary

| Step | Status | Notes |
|---|---|---|
| 1. Commit uncommitted changes on main | ✅ Done | Committed as `da2770d` |
| 2. Create feature branch | ✅ Done | `feat/project-optimisation` |
| 3. Create docs/PROJECT_OPTIMISATION.md | ✅ Done | Full optimization plan document |
| 4. Apply targeted code fixes | ✅ Done | Committed as `cadc73a`, 80 tests pass |
| 5. Split main.rs into modules | ⬜ Not started | Next step |
| 6. Verify compilation and tests | ⬜ Not started | After module split |
| 7. Commit all changes | ⬜ Not started | Final step |

---

## Completed Work Detail

### Step 1–2: Git Setup

- Committed the pre-existing uncommitted changes on `main` (async_trait import + tool_aliases field) as `da2770d`.
- Created branch `feat/project-optimisation` from that commit.

### Step 3: docs/PROJECT_OPTIMISATION.md

Created at `docs/PROJECT_OPTIMISATION.md`. Contains:
- Full module split plan with target file layout (25 modules)
- Security fixes: server auth, execute_bash hardening, prompt limits
- Reliability fixes: graceful shutdown, cache eviction, telemetry mutex
- Build/CI fixes: Cargo.toml metadata, .gitignore, CI security-check
- Deferred items list (blocking I/O, semantic retrieval, cross-platform CI, rustdoc, etc.)

### Step 4: Targeted Code Fixes (ALL APPLIED, NOT YET COMMITTED)

All changes are in the working tree on `feat/project-optimisation`. `cargo check` passes clean, `cargo test` passes all 80 tests.

#### Fix 1: Cargo.toml metadata
**File:** `Cargo.toml`
**Change:** Added `rust-version = "1.85"`, `description`, `license = "MIT"`, `repository`, `authors`, `keywords`, `categories`.

#### Fix 2: .gitignore expansion
**File:** `.gitignore`
**Change:** Added `.DS_Store`, `*.swp`, `*.swo`, `*.bak`, `.idea/`, `.vscode/` to existing entries.

#### Fix 3: CI security-check
**File:** `Makefile`
**Change:** Changed `ci` target from `fmt-check check lint test quality-gate` to `fmt-check check lint test quality-gate security-check`. This ensures `cargo audit` and secret scanning run on every PR, not just releases.

#### Fix 4: execute_bash command chaining hardening
**File:** `src/main.rs` — `is_read_only_command()` function
**Change:** Added `contains_command_chaining()` helper that detects `;`, `&&`, `||`, `|`, `$(`, and backtick characters. `is_read_only_command()` now returns `false` if the command has a read-only prefix but also contains chaining operators. This prevents bypass attacks like `ls; rm -rf /` from being auto-allowed.

New function added:
```rust
fn contains_command_chaining(command: &str) -> bool {
    for pattern in &[";", "&&", "||", "|", "$(", "`"] {
        if command.contains(pattern) {
            return true;
        }
    }
    false
}
```

#### Fix 5: Server bearer token authentication
**File:** `src/main.rs` — `ServerState`, `check_server_auth()`, `handle_server_ask()`, `handle_a2a_ping()`
**Changes:**
- Added `auth_token: Option<String>` field to `ServerState`.
- Added `check_server_auth()` function that checks `Authorization: Bearer <token>` header against `ZAVORA_SERVER_AUTH_TOKEN` env var. When the env var is not set, auth is disabled (backward compatible). When set, all endpoints except `/healthz` require the token.
- Updated `handle_server_ask()` and `handle_a2a_ping()` signatures to accept `headers: axum::http::HeaderMap` and call `check_server_auth()` before processing.
- `run_server()` reads `ZAVORA_SERVER_AUTH_TOKEN` from env and passes it to `ServerState`.

#### Fix 6: Runner cache eviction
**File:** `src/main.rs` — `ServerState`, `get_or_build_server_runner()`
**Changes:**
- Added `runner_cache_max: usize` field to `ServerState` (default 64 via `RuntimeConfig`).
- Updated `get_or_build_server_runner()` to evict the oldest cache entry when `cache.len() >= state.runner_cache_max` before inserting a new entry. Logs eviction via `tracing::info`.

#### Fix 7: Prompt size limit
**File:** `src/main.rs` — `RuntimeConfig`, `enforce_prompt_limit()`, ask/workflow/server paths
**Changes:**
- Added `max_prompt_chars: usize` to `RuntimeConfig` (default 32,000).
- Added `server_runner_cache_max: usize` to `RuntimeConfig` (default 64).
- Added `enforce_prompt_limit()` function that returns an error if `prompt.len() > max_chars`.
- Inserted `enforce_prompt_limit()` calls in:
  - `Commands::Ask` path (after `prompt.join(" ")`, before guardrail)
  - `Commands::Workflow` path (after `prompt.join(" ")`, before guardrail)
  - `handle_server_ask()` (after empty check, before guardrail)

#### Fix 8: Graceful server shutdown
**File:** `src/main.rs` — `run_server()`, new `shutdown_signal()`
**File:** `Cargo.toml` — added `"signal"` to tokio features
**Changes:**
- Added `shutdown_signal()` async function that uses `tokio::select!` to wait for either `tokio::signal::ctrl_c()` (all platforms) or `SIGTERM` (unix only via `tokio::signal::unix`).
- Changed `axum::serve(listener, router).await` to `axum::serve(listener, router).with_graceful_shutdown(shutdown_signal()).await`.
- Added `"signal"` feature to tokio dependency in Cargo.toml.

#### Fix 9: Telemetry write safety
**File:** `src/main.rs` — `TelemetrySink`
**Changes:**
- Added `file_lock: Arc<std::sync::Mutex<()>>` field to `TelemetrySink`.
- Updated `TelemetrySink::new()` to initialize the mutex.
- Updated `append_event_line()` to acquire `self.file_lock.lock()` (with poison recovery via `unwrap_or_else(|e| e.into_inner())`) before opening/writing/closing the file. The lock is held only for the duration of a single line write.

#### Fix 10: Misc cleanup
**File:** `src/main.rs`
- Removed unused `use async_trait::async_trait;` import (was causing compile error since `async_trait` crate is not in Cargo.toml).
- Added `#[allow(dead_code)]` to `tool_aliases` field on `McpServerConfig` (reserved for future use).
- Added `tool_aliases: HashMap::new()` to all 4 test `McpServerConfig` constructions.
- Added `max_prompt_chars: 32_000` and `server_runner_cache_max: 64` to test `base_cfg()`.

---

## Verification State

```
$ cargo check    → ✅ Clean (0 errors, 0 warnings)
$ cargo test     → ✅ 80 passed, 0 failed, 0 ignored
```

---

## Remaining Work

### Step 5: Module Split (NOT STARTED)

This is the largest remaining task. The plan from `docs/PROJECT_OPTIMISATION.md` calls for splitting the 8,911-line `src/main.rs` into ~20 focused module files with a `src/lib.rs` declaring all modules.

Target module layout:
```
src/
├── main.rs          # Entry point: main(), run_cli()
├── lib.rs           # Module declarations and re-exports
├── cli.rs           # Cli, Commands, subcommand enums, ValueEnum types
├── config.rs        # RuntimeConfig, ProfilesFile, ProfileConfig, resolution
├── error.rs         # ErrorCategory, categorize_error, format_cli_error, redaction
├── provider.rs      # detect_provider, resolve_model, validate_model_for_provider
├── session.rs       # Session service, CRUD commands, SQLite helpers
├── runner.rs        # build_runner variants, resolve_runtime_tools, tool confirmation
├── tools/
│   ├── mod.rs       # build_builtin_tools, shared types, workspace path policy
│   ├── fs_read.rs   # FsReadRequest, fs_read_tool_response
│   ├── fs_write.rs  # FsWriteRequest, fs_write_tool_response
│   ├── execute_bash.rs  # ExecuteBashRequest, policy, execute_bash_tool_response
│   └── github_ops.rs   # GitHub CLI operations, auth preflight
├── workflow.rs      # Workflow agents, route classifier, templates
├── retrieval.rs     # RetrievalService trait, local/semantic backends
├── mcp.rs           # MCP server selection, auth, tool discovery
├── server.rs        # Axum server, handlers, runner cache, auth
├── chat.rs          # Interactive chat, slash commands, model picker
├── streaming.rs     # AuthorTextTracker, text dedup, stream suffix
├── telemetry.rs     # TelemetrySink, TelemetrySummary, report
├── guardrail.rs     # Guardrail modes, term matching, apply_guardrail
├── eval.rs          # Eval dataset, harness, benchmark, report
├── doctor.rs        # run_doctor, run_migrate
├── profiles.rs      # run_profiles_list, run_profiles_show
└── agents.rs        # Agent catalog, selection, list/show/select
```

Key considerations for the split:
- Most types need `pub(crate)` visibility
- `RuntimeConfig`, `Provider`, `SessionBackend`, `GuardrailMode`, `ToolConfirmationMode`, `RetrievalBackend`, `WorkflowMode` are used across many modules — they live in `cli.rs` and `config.rs`
- `TelemetrySink` is passed to nearly every command — lives in `telemetry.rs`
- Workspace path policy (`enforce_workspace_path_policy`, `fs_read_workspace_root`) is shared between `fs_read` and `fs_write` — lives in `tools/mod.rs`
- Tests should move to their respective modules as `#[cfg(test)] mod tests { ... }`
- The `#[cfg(test)] fn resolve_runtime_config()` helper is used by many config tests — stays in `config.rs`

### Step 6: Verify compilation and tests
After module split, run `cargo check`, `cargo test`, `cargo clippy`.

### Step 7: Commit
Commit the targeted fixes and module split as separate commits:
1. `feat: apply targeted code fixes from project review` (the current uncommitted changes)
2. `refactor: split main.rs into focused modules` (the module split)

---

## Files Modified (Uncommitted)

| File | Change Type |
|---|---|
| `docs/PROGRESS.md` | Created (this file) |

## Files Modified (Committed in `cadc73a`)

| File | Change Type |
|---|---|
| `Cargo.toml` | Modified (metadata, tokio signal feature) |
| `.gitignore` | Modified (expanded) |
| `Makefile` | Modified (security-check in ci target) |
| `src/main.rs` | Modified (all code fixes) |
| `docs/PROJECT_OPTIMISATION.md` | Created |
