# Tasks: ADK Crate Adoption for Zavora-CLI

## Phase 1 — Drop-in Integrations

### Task 1: Skill System (`adk-skill`)
- [ ] Add `adk-skill` path dependency to `Cargo.toml`
- [ ] Add `.with_auto_skills(".", SkillInjectorConfig::default())` to `Runner::builder()` in `src/runner.rs`
- [ ] Create `.zavora/skills/` directory with a sample skill file
- [ ] Add `Commands::Skills` with `SkillCommands::List` to `src/cli.rs`
- [ ] Wire `zavora skills list` in `src/main.rs` — load skill index, print names + descriptions + triggers
- [ ] Add skill system to system prompt tool guidelines in `src/runner.rs`
- [ ] Update README with skills documentation
- [ ] Verify build and tests

### Task 2: Semantic Memory (`adk-memory`)
- [ ] Add `adk-memory` path dependency with `sqlite-memory` feature to `Cargo.toml`
- [ ] Create `SqliteMemoryService` in `src/runner.rs` `build_runner_with_session_service()`, pass to builder via `.memory_service()`
- [ ] Implement one-time migration from `.zavora/memory.json` to `.zavora/memory.db` on first run
- [ ] Update `MemoryAgentTool` in `src/agents/tools.rs` to use `ToolContext::search_memory()` instead of direct JSON I/O
- [ ] Remove or deprecate `src/agents/memory.rs` JSON file operations
- [ ] Verify `/memory recall|remember|forget` chat commands still work
- [ ] Update README
- [ ] Verify build and tests

### Task 3: OpenTelemetry (`adk-telemetry`)
- [ ] Add `adk-telemetry` path dependency to `Cargo.toml`
- [ ] Call `adk_telemetry::init_telemetry("zavora-cli")` in `src/main.rs` before `run_cli()`
- [ ] Call `adk_telemetry::shutdown_telemetry()` at exit
- [ ] Update `TelemetrySink` in `src/telemetry.rs` to emit OpenTelemetry events via `tracing::info!` alongside JSONL
- [ ] Verify `OTEL_EXPORTER_OTLP_ENDPOINT` env var enables OTLP export
- [ ] Verify `zavora telemetry report` still works (reads JSONL fallback)
- [ ] Update README
- [ ] Verify build and tests

---

## Phase 2 — Enhanced Safety

### Task 4: Guardrails (`adk-guardrail`)
- [ ] Add `adk-guardrail` path dependency to `Cargo.toml`
- [ ] Replace `apply_guardrail()` in `src/guardrail.rs` with `PiiRedactor::new().redact()` for redact mode
- [ ] Replace keyword matching with `ContentFilter::blocked_keywords()` for block mode
- [ ] Add `ContentFilter::harmful_content()` for the `block` guardrail mode
- [ ] Add `guardrail_pii_redaction` bool to `ProfileConfig` and `RuntimeConfig`
- [ ] Verify `--guardrail-input-mode` and `--guardrail-output-mode` CLI flags still work
- [ ] Update README
- [ ] Verify build and tests

### Task 5: Plugin System + File History (`adk-plugin`)
- [ ] Add `adk-plugin` path dependency to `Cargo.toml`
- [ ] Create `src/file_history.rs` with `snapshot_file()` and `restore_last_snapshot()` functions
- [ ] Implement file history plugin using `PluginConfig::before_tool` hook — snapshot files before `fs_write`/`file_edit`
- [ ] Store snapshots in `.zavora/file_history/<path_hash>/<timestamp>.snapshot`, max 20 per file
- [ ] Create `PluginManager`, register file-history plugin, pass to `Runner::builder().plugin_manager()`
- [ ] Add `ChatCommand::Undo` to `src/chat.rs` — calls `restore_last_snapshot()`
- [ ] Add `/undo` to chat help display
- [ ] Add `file_history_enabled` to `ProfileConfig` (default: true)
- [ ] Update README
- [ ] Verify build and tests

---

## Phase 3 — New Capabilities

### Task 6: Browser Automation (`adk-browser`)
- [ ] Add `adk-browser` as optional dependency behind `browser` feature flag
- [ ] Create lazy `BrowserSession` via `OnceCell` (initialized on first browser tool use)
- [ ] Register 8 browser tools in `build_builtin_tools()` (feature-gated): Navigate, Click, TypeText, Extract, Screenshot, Wait, Evaluate, Cookies
- [ ] Wrap all browser tools with `ConfirmingTool` (require confirmation)
- [ ] Add browser session cleanup on chat exit
- [ ] Add browser tools to system prompt
- [ ] Update README with browser feature documentation
- [ ] Verify build with and without `browser` feature

### Task 7: Code Sandbox (`adk-code` + `adk-sandbox`)
- [ ] Add `adk-code` and `adk-sandbox` as optional dependencies behind `sandbox` feature flag
- [ ] Create `code_execute` tool using `SandboxTool::new(ProcessBackend::new(config))`
- [ ] Configure sandbox: 30s timeout, temp directory only, no network
- [ ] Register in `build_builtin_tools()` (feature-gated)
- [ ] Wrap with `ConfirmingTool` (require confirmation)
- [ ] Add to system prompt
- [ ] Update README
- [ ] Verify build with and without `sandbox` feature

### Task 8: RAG Pipeline (`adk-rag`)
- [ ] Add `adk-rag` as optional dependency behind `rag` feature flag with `inmemory` feature
- [ ] Replace `build_retrieval_service()` in `src/retrieval.rs` with `RagPipeline::builder()`
- [ ] Implement `augment_prompt_with_rag()` using `pipeline.query()`
- [ ] Add `Commands::Rag` with `RagCommands::Ingest { path }` to `src/cli.rs`
- [ ] Wire `zavora rag ingest <path>` in `src/main.rs`
- [ ] Verify `--retrieval-backend` and `--retrieval-doc-path` CLI flags still work
- [ ] Update README
- [ ] Verify build with and without `rag` feature
