# Changelog

All notable changes to this project are documented in this file.

The format is based on Keep a Changelog and this project follows Semantic Versioning.

## [Unreleased]

## [1.1.3] — 2026-02-15

### Added

- Syntax-highlighted diffs for fs_write confirmations (syntect + similar crates)
  - base16-ocean.dark theme, truecolor RGB backgrounds for added/removed lines
  - Proper unified diff via `similar::TextDiff` with line numbers in gutter
  - Language detection from file extension; graceful fallback to plain text
- Tool result display after execution
  - `execute_bash`: stdout shown directly, stderr in red
  - `fs_write`: `✓ wrote <path>` confirmation on success
- `/usage` diagnostics: events count, raw chars, overhead breakdown, API tokens
- `/agent` chat command: trust all tools for the session with warning prompt
- Comprehensive system prompt with `<system_context>`, `<operational_directives>`,
  `<tone>`, `<coding_standards>`, `<tool_guidelines>`, `<response_format>`, `<rules>`
- `fs_read` display-only mode: shows path but auto-approves (no y/n prompt)
- Tool transparency: actions always visible even when trusted (Q CLI pattern)
- Terminal-width-aware banner and tip boxes (capped at 120 columns)
- Default to chat mode: bare `zavora-cli` enters interactive chat

### Changed

- Context windows updated to factual values from official model cards
  - Model-level lookup (`model_context_window`) with provider fallback
  - GPT-5 family: 400K, Claude Sonnet 4: 1M, DeepSeek: 128K, Groq Scout: 131K
- Context usage now counts FunctionCall args and FunctionResponse payloads
- Added 1500-token overhead estimate for system prompt + tool declarations
- Prompt shows `<1%` instead of `0%` for small utilization values
- System prompt: "don't repeat file contents after tool writes them"

### Fixed

- OpenAI 400 Bad Request on multi-turn: `before_model_callback` restores
  `role: "function"` on FunctionResponse parts (ADK maps all to "model")
- Tool confirmation: injects `"approved": true` into args when user approves
- Context usage was stuck at 0%: only Part::Text was counted, missing all
  FunctionCall/FunctionResponse content

## [1.1.2] — 2026-02-15

### Added

- Streaming markdown renderer using winnow 0.7 + crossterm (replaces line-based renderer)
  - Magenta+bold headings, green code blocks, DarkGrey blockquotes
  - Terminal-width-aware word wrapping with column tracking
- Interactive tool confirmation with file diff preview
  - Shows colored diff (red removals, green additions) for fs_write
  - Shows `$ command` for execute_bash
  - `y` to approve, `n` to deny, `t` to trust tool for the session
- Readline support via rustyline — arrow key history, line editing, Ctrl-C
- 2026 model catalog: GPT-5.3-Codex, Claude Opus 4.6, Gemini 3 Pro, Llama 4

### Changed

- Default OpenAI model: gpt-4.1 (was gpt-4o-mini)
- Default Ollama model: llama4 (was llama3.2)
- Default log level: error (was warn) — no more WARN traces in normal mode
- Context window defaults updated for 2026 models
- Runner event errors no longer crash the session — logged and continued

### Removed

- Old line-based MarkdownRenderer from theme.rs

## [1.1.1] — 2026-02-15

### Fixed

- Context usage now computed from real session events (was always None)
- `/delegate` now runs isolated sub-agent prompt (was placeholder message)
- StubTool moved from production code to test module
- Checkpoint store persisted to `.zavora/checkpoints.json` across CLI restarts
- Added `todo_list` agent tool so the model can create/update todos during execution

## [1.1.0] — 2026-02-15

Phase 2: Q CLI Parity + UX (Sprints 9–11)

### Added

- Tool aliases with wildcard allow/deny filtering (`tool_policy.rs`) (#37)
- Hook lifecycle system with 5 hook points and pre-tool blocking (`hooks.rs`) (#38)
- MCP diagnostics with server state, latency, and auth hints (`mcp.rs`) (#39)
- Context usage tracking with token estimation and budget warnings (`context.rs`) (#40)
- Manual `/compact` command and auto-compaction via ADK EventsCompactionConfig (`compact.rs`) (#41)
- Checkpoint save/list/restore for conversation snapshots (`checkpoint.rs`) (#42)
- Tangent mode with enter/exit/tail for exploratory branching (`checkpoint.rs`) (#42)
- Todo list persistence with file-based CRUD in `.zavora/todos/` (`todos.rs`) (#43)
- Delegate sub-agent experiment data model (`todos.rs`) (#43)
- Unified theme with mode indicators in prompt (`theme.rs`) (#44)
- Command palette with fuzzy prefix matching and did-you-mean suggestions (`theme.rs`) (#44)
- First-run onboarding detection and help (`theme.rs`) (#44)
- Parity benchmark suite with 12 scenarios and weighted scorecard (`benchmark.rs`) (#45)
- Parity matrix document — 98.5% parity (33/34 capabilities Met) (#46)
- Differentiation roadmap with 6 current and 4 planned differentiators (#46)
- v1.1.0 GA sign-off with release gates, rollback playbook, migration guidance (#47)

### Changed

- `/usage` now shows token breakdown instead of help text
- `/help` updated with all new slash commands
- Unknown commands now show fuzzy suggestions
- Runner auto-compaction wired via `auto_compact_enabled` config (default: true)

## [1.0.0] — 2026-02-15

### Added

- Initial ADK-Rust CLI scaffold with provider-aware runtime.
- Workflow modes: `single`, `sequential`, `parallel`, and `loop`.
- Release-planning command for release-sliced execution plans.
- CI workflow and tag-based release workflow.
- Agile release cycle documentation and release quality gates.
- Selectable session backend with SQLite persistence support.
- Deterministic workflow tests using ADK `MockLlm`.
- `migrate` command for SQLite session schema setup.
- `sessions list/show` commands for session inspection.
