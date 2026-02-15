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
| 5. Split main.rs into modules | ✅ Done | Committed as `dc3d249`, 26 files changed |
| 6. Verify compilation and tests | ✅ Done | 80 tests pass, 0 warnings, clean build |
| 7. Commit all changes | ✅ Done | All commits on `feat/project-optimisation` |

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

## Completed: Module Split (Step 5)

Committed as `dc3d249`. Split the 9,032-line `src/main.rs` into 20 focused modules:

```
src/
├── main.rs              (359 lines)  Entry point + command dispatch
├── lib.rs               (22 lines)   Module declarations
├── cli.rs               (395 lines)  CLI types, command enums, ValueEnum types
├── config.rs            (546 lines)  RuntimeConfig, ProfilesFile, resolution
├── error.rs             (132 lines)  ErrorCategory, format_cli_error, redaction
├── telemetry.rs         (229 lines)  TelemetrySink, TelemetrySummary, report
├── guardrail.rs         (167 lines)  Guardrail modes, term matching, apply
├── eval.rs              (316 lines)  Eval dataset, harness, benchmark, report
├── retrieval.rs         (258 lines)  RetrievalService trait, backends, augment
├── provider.rs          (149 lines)  detect_provider, resolve_model, validate
├── streaming.rs         (391 lines)  AuthorTextTracker, text dedup, run_prompt
├── mcp.rs               (258 lines)  MCP discovery, server selection
├── session.rs           (345 lines)  Session CRUD, SQLite helpers
├── runner.rs            (314 lines)  build_runner variants, tool confirmation
├── workflow.rs          (446 lines)  Workflow agents, route classifier, templates
├── server.rs            (453 lines)  Axum server, handlers, cache, auth
├── chat.rs              (678 lines)  Interactive chat, slash commands, model picker
├── doctor.rs            (112 lines)  run_doctor, run_migrate
├── profiles.rs          (97 lines)   run_profiles_list, run_profiles_show
├── agents.rs            (131 lines)  Agent catalog list/show/select
├── tools/mod.rs         (82 lines)   build_builtin_tools, tool registration
├── tools/fs_read.rs     (360 lines)  FsReadRequest, workspace policy
├── tools/fs_write.rs    (410 lines)  FsWriteRequest, patch mode
├── tools/execute_bash.rs(358 lines)  ExecuteBashRequest, policy
├── tools/github_ops.rs  (323 lines)  GitHub CLI operations
└── tests.rs             (1954 lines) All 80 tests
```

Total: 9,285 lines across 26 files.

## Verification (Step 6)

```
$ cargo build    → ✅ Clean (0 errors, 0 warnings)
$ cargo check    → ✅ Clean (0 errors, 0 warnings)
$ cargo test     → ✅ 80 passed, 0 failed, 0 ignored
```

## Commit History on `feat/project-optimisation`

| Commit | Message |
|---|---|
| `dc3d249` | refactor: split monolithic main.rs into 20 focused modules |
| `bd6e363` | docs: add detailed implementation progress tracker |
| `cadc73a` | feat: apply targeted code fixes from project review |
| `da2770d` | chore: add async_trait import and tool_aliases field to McpServerConfig |

## Files Modified

### Committed in `dc3d249` (module split)

26 files changed, 9,252 insertions, 8,999 deletions:
- `src/main.rs` — rewritten as thin entry point
- 20 new module files created (see layout above)
- `src/tools/` directory with 5 files
- `src/tests.rs` — all 80 tests extracted

### Committed in `cadc73a` (code fixes)

| File | Change Type |
|---|---|
| `Cargo.toml` | Modified (metadata, tokio signal feature) |
| `.gitignore` | Modified (expanded) |
| `Makefile` | Modified (security-check in ci target) |
| `src/main.rs` | Modified (all code fixes) |
| `docs/PROJECT_OPTIMISATION.md` | Created |
