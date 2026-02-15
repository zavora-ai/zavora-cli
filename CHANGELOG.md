# Changelog

All notable changes to this project are documented in this file.

The format is based on Keep a Changelog and this project follows Semantic Versioning.

## [Unreleased]

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
