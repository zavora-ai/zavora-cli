# Changelog

All notable changes to this project are documented in this file.

The format is based on Keep a Changelog and this project follows Semantic Versioning.

## [2.0.0] — 2026-08-16

### Added

- Capability enablement the agent can drive: `capability_status` reports, per capability, its risk, specialist agent, and whether each MCP server is installed and configured; `capability_enable` installs and configures behind an approval that names the exact install commands. The model supplies only a capability id validated against the built-in set, every install command comes from the curated catalogue, and installs run as an argument vector rather than through a shell.
- Complete MCP server catalogue: every server a capability names now has an entry, so each capability can actually be provisioned. Four capabilities previously named only servers that did not exist.
- Mid-session capability activation: because a sealed tool surface is one-shot, enabling marks it stale and the workspace re-resolves and rebuilds between turns, reporting how many servers answered rather than assuming.
- Terminal-aware keyboard action registry: every chord is declared once with its description, category, and footer priority, and bindings adapt to what the host terminal actually delivers. Footer hints and `/keys` are generated from the same table the key handler dispatches through, so neither can name a chord that is unbound or undeliverable.
- Response-level and line-level transcript navigation, a configurable mouse-wheel step (`/mouse speed`), and `/keys`.
- Retained full-screen Ratatui workspace with responsive conversation and activity layouts, a multiline Unicode-aware composer, BUILD/PLAN mode, keyboard overlay, in-place tool approvals, and automatic bottom-following output.
- Terminal-native Markdown presentation with headings, emphasis, inline code, fenced code surfaces, and lightweight Rust syntax treatment.
- Tool activity correlation by provider call ID so parallel calls with the same tool name remain distinct.
- Versioned automation surface (`--output-format text|json|stream-json`) with clean stdout, repeatable `--file` inputs, automatic piped stdin, and stable exit codes. See `docs/HEADLESS.md`.
- Cross-CLI plugin system normalizing Zavora, Codex, Claude, Gemini, Grok, and OpenCode packages, with `plugins list|validate|install|doctor` and namespaced skill, agent, command, and MCP contribution.
- Portable skills at `.agents/skills/<name>/SKILL.md` with install, link, update, enable, disable, and uninstall, injected through ADK-Rust's plugin runtime for every provider.
- Live capability model with five categories and MCP recipes, reporting `configured`, `connected`, and `authorized` separately (`capabilities list|search|info`).
- Comment-preserving MCP configuration via `mcp catalog`, `mcp add`, and `mcp remove`, plus `mcp protocol --json` reporting remaining transport and authorization gates.
- Project instruction discovery for the `AGENTS.md`, `GEMINI.md`, and `CLAUDE.md` families with scope resolution and `AGENTS.override.md` precedence (`instructions list|show`).
- Five specialist subagents — `artifact_agent`, `developer_agent`, `research_agent`, `operations_agent`, `reviewer_agent`.
- Single tool-surface enforcement point: tools are classified as read-only, mutating, or network-egress, then filtered and wrapped once. The sealed surface cannot be extended afterwards.
- Lifecycle hooks now run: a `pre_tool` hook exiting with code 2 blocks the call, and `post_tool` hooks observe the result.
- Multi-strategy automatic compaction — stale-tool-result elision and file-read deduplication first, LLM summarization as fallback, bounded repeat as escalation — in both the workspace and the classic shell.
- `doctor` reports the external binaries the enabled features require, including `rg` and language servers.
- Release gates as executable scripts: `scripts/check_clean_clone.sh`, `scripts/check_wiring.sh`, and `scripts/check_docs.sh`, all wired into `make ci`.

### Changed

- Interactive TTY chat now opens the full-screen workspace; redirected, dumb-terminal, and `ZAVORA_CLASSIC=1` sessions retain the line-oriented interface.
- Console tracing is suppressed while the alternate-screen renderer owns the terminal, while structured OTLP telemetry remains available.
- TUI text follows the terminal foreground/background instead of assuming a dark theme.
- Migrated the complete runtime to ADK-Rust 2.0 and `rmcp` 3.1.
- Stdio MCP tool discovery prefers the `2026-07-28` `server/discover` lifecycle and falls back to `2025-11-25` for compliant legacy servers, and now retries with bounded exponential backoff.
- ADK-Rust dependencies are consumed from crates.io, so a clean checkout builds with no sibling repositories. Use `make local-adk` for local ADK development.
- Rust MSRV is now 1.95 and the crate uses the Rust 2024 edition. The toolchain is pinned by `rust-toolchain.toml`.
- The workspace transcript and prompt history are bounded, with a single visible marker recording any elision, and each message renders once per revision rather than once per frame.

### Fixed

- Scroll position is anchored to the conversation rather than to the newest line. Counting back from the end meant a view scrolled away from the tail drifted at exactly the rate output streamed in, carrying earlier output out of reach.
- The visible transcript window is sliced in `usize` space instead of relying on `Paragraph::scroll`, whose `u16` offset made the top of a session past 65535 rendered rows unreachable.
- The scroll indicator moved to the bottom border: as a top-edge title it consumed a content row whenever the view detached, so a one-line scroll moved the view two lines.
- The mouse wheel is claimed by the workspace. Left to the terminal in the alternate screen it scrolls the terminal's own buffer, displacing the drawn frame so subsequent diffed redraws land at the wrong row and interleave old and new text.
- `Ctrl+L` repaints the screen; clearing the conversation now takes a second press. The conventional reflex for a corrupted screen previously destroyed the transcript.
- The terminal is handed back on a panic and on a signal, not only on exit. A killed process previously left mouse reporting enabled, so every pointer movement printed escape sequences into the shell.
- The welcome screen starts at the top of the transcript pane instead of floating in its middle.
- Specialist agents are called as `AgentTool`s rather than reached through `transfer_to_agent`, which handed over the turn and left an unpaired function call that the OpenAI Responses API rejects.

- Input and output guardrails remain active in retained TUI sessions, including buffered output for block and redact modes.
- Raw mode and the alternate screen are restored when terminal initialization fails.
- Read-only shell auto-approval no longer discloses secrets: `env` and `printenv` were removed from the allowlist, and every path argument is checked against the same containment policy `fs_read` applies.
- A shell command that times out or is cancelled is killed along with its process group, and a timed-out command is never retried.
- The shell tool no longer uses a login shell, which had re-sourced profile files and undone the PATH and `LD_PRELOAD` protections the validators enforce.
- Browser tools and `web_fetch` can no longer bypass confirmation and allow/deny policy.
- Model-supplied `approved` and `allow_dangerous` arguments are stripped before the enforcement decision.
- The system prompt no longer advertises `file_search_agent`, `sequential_agent`, or `quality_agent`, none of which was registered. The `/orchestrate` command and its placeholder agents were removed rather than shipped as no-ops.
- A malformed skill, plugin, or agent manifest is reported on every surface instead of being presented as an empty capability set.
- An unreachable MCP server is reported on the active surface rather than only in a log sink.
- OAuth authorization now includes and verifies a `state` parameter, and the filesystem token fallback is written owner-only.
- The HTTP server refuses a non-loopback bind without an authentication token, compares bearer tokens in constant time, and withholds workspace detail from unauthenticated health callers.
- Closed six RUSTSEC advisories (quinn-proto, rustls-webpki ×3, time, crossbeam-epoch) and replaced a yanked transitive dependency.

## [2.0.0-dev] — 2026-07-14

### Added

- Independent worker and planner provider/model roles across CLI flags, environment variables, profiles, and interactive commands.
- OpenAI-first model catalog with the available 1M and 10M shared daily pools, role guidance, and retired-model filtering.
- Bounded `plan_work` agent tool: a strong planning agent can advise the worker for complex work without receiving mutation tools.
- `/models`, `/worker`, `/planner`, and `/planner-provider` chat commands plus the `zavora-cli models` catalog command.
- OS credential-vault storage for setup API keys.
- Unicode-safe previews and truncation for streamed content, hook output, compaction, web fetches, and tool confirmations.
- Kiro requirements, design, and implementation task specification in `.kiro/specs/v2-upgrade`.

### Changed

- Migrated the complete runtime to ADK-Rust 2.0 and `rmcp` 3.1.
- Stdio MCP tool discovery prefers the `2026-07-28` `server/discover`
  lifecycle and falls back to `2025-11-25` for compliant legacy servers.
- OpenAI is now the default provider. Routine work defaults to `gpt-5.4-mini-2026-03-17`; planning defaults to `gpt-5.6-sol` with four calls per process.
- OpenAI now uses the ADK-Rust v2 Responses API client.
- Ralph now shares the v2 worker, planner, session, and tool runtime instead of pulling a duplicate ADK 0.5 dependency graph.
- Replaced the large startup wordmark and model-generated greeting with a compact workspace, session, and model-routing surface.
- Provider/model changes preserve the session and can update worker and planner independently.
- Setup no longer creates a sample skill in every working directory.
- Rust MSRV is now 1.95 and the crate uses the Rust 2024 edition. The
  toolchain is pinned by `rust-toolchain.toml`.

### Fixed

- Setup credentials are now read by the runtime and are no longer written in plaintext TOML.
- Removed an obsolete OpenAI role-repair callback that conflicted with the v2 model pipeline.
- Agent instructions no longer direct the model to stage every file, commit, or push without an explicit developer request.
- MCP client/server content conversion now follows the current `ContentBlock` API.
- ANSI styling is omitted for `NO_COLOR`, dumb terminals, and redirected output.

## [1.2.0] — 2026-04-05

### Added

- **adk-skill** — auto-discovers `.skills/`, `.claude/skills/`, `~/.zavora/skills/`; `zavora skills list` CLI; tested with 17 Anthropic skills
- **adk-memory** — SQLite FTS5 semantic memory via `SqliteMemoryService`; shared singleton for Runner + chat commands; `/memory recall` (empty = list all), `/memory remember`, `/memory forget`
- **adk-telemetry** — composable OTLP layer via `build_otlp_layer()` + console tracing on same subscriber; `shutdown_telemetry()` on exit
- **adk-guardrail** — `PiiRedactor` (emails, phones, SSNs, credit cards) + `ContentFilter` (blocked keywords); redact mode chains PII then custom terms
- **File history** — snapshots files before `fs_write` and `file_edit` (direct hooks); max 20 per file; `/undo` command restores last modified file
- **adk-browser** — 40+ browser automation tools via WebDriver (feature: `browser`); lazy headless session; cleanup on chat exit
- **adk-sandbox** — sandboxed code execution via ProcessBackend (feature: `sandbox`); Python, Node.js, Rust
- **adk-rag** — RAG pipeline with InMemoryVectorStore + bag-of-words embedding (feature: `rag`); `zavora rag ingest <path>` CLI; RecursiveChunker (512/100)

### Changed

- Memory: single `OnceLock` singleton initialized in `main.rs`, shared by Runner (`.memory_service()`) and chat/tool commands — replaces hand-rolled JSON
- Guardrail: `adk-guardrail` PiiRedactor + ContentFilter replaces hand-rolled regex
- Runner: `with_auto_skills_mut()` (borrow-safe) for skill injection
- Telemetry: `build_otlp_layer()` composes with existing subscriber (no takeover)
- Orchestrator: async memory API instead of direct JSON I/O
- Architecture: memory singleton wired into Runner, browser cleanup on exit, removed unused adk-plugin dep

### Previous

- **file_edit** tool — surgical `old_string → new_string` text replacement with unified diff output, closest-match hints on miss, line-ending preservation
- **glob** tool — gitignore-aware file pattern search via `ignore` crate, structured output with truncation
- **grep** tool — ripgrep wrapper with output modes (content/files_with_matches/count), context lines, pagination, `grep -rn` fallback
- **web_fetch** tool — HTTP fetch with HTML→markdown conversion, domain blocklist (SSRF protection), JSON pretty-print (feature-gated: `web-fetch`)
- **lsp** tool — Language Server Protocol integration with 9 operations (goToDefinition, findReferences, hover, documentSymbol, workspaceSymbol, goToImplementation, prepareCallHierarchy, incomingCalls, outgoingCalls), lazy server lifecycle, 7 language support (feature-gated: `lsp`)
- **MCP server mode** — `zavora mcp serve` exposes all built-in tools as an MCP server over stdio via `rmcp`
- **MCP stdio client** — connect to local MCP servers via stdio transport (`command`/`args`/`env` config), alongside existing HTTP
- **Layered permission system** — `PermissionRules` with `always_allow`/`always_deny`/`always_ask` glob patterns, content-aware matching (`execute_bash:git status*`), `/allow` and `/deny` slash commands
- **Bash security pipeline** — 20 validation checks replacing flat denied-patterns: command substitution, shell metacharacters, dangerous variables, unicode whitespace, brace expansion, proc/environ access, IFS injection, Zsh dangerous commands, and more
- **Parallel tool execution** — `ToolExecutionStrategy::Auto` runs read-only tools concurrently via ADK ergonomics
- **Fork sub-agents** — `fork_sub_agent()` with fresh session, 5-minute timeout, optional file context, automatic session cleanup
- **Multi-strategy compaction** — snip (remove stale/large/duplicate tool results without LLM), auto mode (snip first → summary fallback), file-read dedup
- **MCP OAuth 2.0** — Authorization Code with PKCE flow, OS keychain token storage via `keyring`, automatic refresh, browser-based authorization (feature-gated: `oauth`)
- **Tool search** — keyword-based tool discovery for large tool sets (auto-enabled when >15 tools); searches names and descriptions, returns matching schemas
- `zavora lsp-init` command to auto-detect and configure language servers

### Changed

- ADK-Rust upgraded from 0.3.2 to 0.5.0 (local path deps for ergonomics fixes)
- Runner uses `Runner::builder()` instead of struct literal (future-proof against new fields)
- Streaming uses `run_str()` instead of `UserId`/`SessionId` newtype conversions
- MCP server uses `SimpleToolContext` instead of 40-line boilerplate
- Tools declare `is_read_only()` and `is_concurrency_safe()` via `FunctionTool` builders
- System prompt updated with tool guidelines for file_edit, glob, grep, web_fetch

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
