# Tasks: ADK Crate Adoption for Zavora-CLI

## Phase 1 — Drop-in Integrations

### Task 1: Skill System (`adk-skill`) ✅
- [x] Add `adk-skill` path dependency to `Cargo.toml`
- [x] Use `with_auto_skills_mut()` in runner (borrow-safe)
- [x] Support `.skills/`, `.claude/skills/`, `~/.zavora/skills/`
- [x] Add `Commands::Skills` with `SkillCommands::List` to `src/cli.rs`
- [x] Wire `zavora skills list` in `src/main.rs`
- [x] Tested with 17 Anthropic skills from github.com/anthropics/skills
- [x] Update README with skills documentation
- [x] Verify build and tests (210 pass)

### Task 2: Semantic Memory (`adk-memory`) ✅
- [x] Add `adk-memory` path dependency with `sqlite-memory` feature
- [x] Create `SqliteMemoryService` + `MemoryServiceAdapter` in runner
- [x] Auto-migrate `.zavora/memory.json` → `.zavora/memory.db`
- [x] Rewrite `agents/memory.rs` to use SQLite (tokio::spawn for SQLx lifetimes)
- [x] Update `MemoryAgentTool` to async SQLite calls
- [x] Update orchestrator to async memory API
- [x] Verify `/memory recall|remember|forget` commands
- [x] Update README
- [x] Verify build and tests (210 pass)

### Task 3: OpenTelemetry (`adk-telemetry`) ✅
- [x] Add `adk-telemetry` path dependency
- [x] Use `init_with_otlp()` when `OTEL_EXPORTER_OTLP_ENDPOINT` is set
- [x] Call `shutdown_telemetry()` at exit
- [x] Console tracing fallback when no OTLP endpoint
- [x] Update README
- [x] Verify build and tests (210 pass)

---

## Phase 2 — Enhanced Safety

### Task 4: Guardrails (`adk-guardrail`) ✅
- [x] Add `adk-guardrail` path dependency
- [x] Replace keyword matching with `ContentFilter::blocked_keywords()`
- [x] Add `PiiRedactor` for email/phone/SSN/credit card redaction
- [x] Redact mode chains PII redaction then custom term redaction
- [x] Existing API surface unchanged (apply_guardrail, GuardrailMode)
- [x] Update README
- [x] Verify build and tests (210 pass)

### Task 5: Plugin System + File History (`adk-plugin`) ✅
- [x] Add `adk-plugin` path dependency
- [x] Create `src/file_history.rs` with snapshot/undo
- [x] Hook `snapshot_file()` into `file_edit` and `fs_write` (overwrite/append)
- [x] Max 20 snapshots per file, oldest pruned
- [x] Add `/undo` chat command
- [x] Update README + help display
- [x] Verify build and tests (210 pass)

---

## Phase 3 — New Capabilities (NOT STARTED)

### Task 6: Browser Automation (`adk-browser`)
- [ ] Add `adk-browser` as optional dependency behind `browser` feature flag
- [ ] Create lazy `BrowserSession` via `OnceCell`
- [ ] Register 8 browser tools (feature-gated)
- [ ] Wrap with `ConfirmingTool`
- [ ] Update README
- [ ] Verify build with and without `browser` feature

### Task 7: Code Sandbox (`adk-code` + `adk-sandbox`)
- [ ] Add as optional dependencies behind `sandbox` feature flag
- [ ] Create `code_execute` tool with `ProcessBackend`
- [ ] Configure: 30s timeout, temp dir only, no network
- [ ] Wrap with `ConfirmingTool`
- [ ] Update README
- [ ] Verify build with and without `sandbox` feature

### Task 8: RAG Pipeline (`adk-rag`)
- [ ] Add as optional dependency behind `rag` feature flag
- [ ] Replace `build_retrieval_service()` with `RagPipeline::builder()`
- [ ] Add `zavora rag ingest <path>` CLI command
- [ ] Update README
- [ ] Verify build with and without `rag` feature

---

## Feedback Filed

- Issue #260: Skill + memory improvements (7 items) → **RESOLVED** by adk-rust team
- Issue #262: Phase 1-2 integration feedback (5 items) → **OPEN**
  1. SQLx async_trait lifetime issues (HIGH)
  2. init_telemetry() subscriber conflict (MEDIUM)
  3. BeforeToolCallback missing tool name/args (MEDIUM)
  4. harmful_content() false positives for dev terms (LOW)
  5. PluginBuilder documentation (LOW)
