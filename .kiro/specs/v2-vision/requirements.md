# Requirements Document

## Introduction

Zavora CLI v2 is a production terminal coding agent built on ADK-Rust 2.0. This spec states the complete v2 vision as a single verifiable contract, superseding the partial view held by the six earlier specs.

The v2 thesis has three parts:

1. **Deliberate model spend.** A cheap worker model owns the conversation, the tools, and the implementation. A stronger planner is a bounded, callable tool, not a co-pilot on every turn. The developer can always see which model is doing what.
2. **A workspace that survives a long session.** A retained full-screen terminal surface keeps the transcript, tool activity, context usage, model roles, and in-place approvals visible for hours, and degrades cleanly to a line-oriented shell, a dumb terminal, or a machine-readable automation stream.
3. **Capability honesty.** Zavora reports capability state from the live runtime. `discovered`, `configured`, `enabled`, `connected`, and `authorized` are distinct states, and no surface may collapse them. A feature that exists in the source but is never invoked at runtime is not a shipped feature.

The third point is the binding constraint of this spec. An audit conducted on 2026-08-15 found that the code is materially healthier than its own paperwork: the build, tests, lint, feature matrix, and crates.io packaging all pass, while a clean checkout cannot build at all, six features are written and never called, the system prompt advertises three tools that do not exist, and the specs' own checkboxes are wrong in both directions. Every requirement below therefore carries a **Current state** line grounded in a command result or a file citation, so this document can be trusted as a release gate rather than a wish list.

## Verification baseline

Established 2026-08-15 with `cargo +1.95` (MSRV is 1.95 per `Cargo.toml:5`; the machine default toolchain was 1.94, which refuses to build).

| Check | Result |
|---|---|
| `cargo +1.95 check --all-targets` | clean |
| `cargo +1.95 check --all-targets --features "web-fetch,lsp,oauth,browser,sandbox,rag,semantic-search,checkpoints"` | clean |
| `cargo +1.95 test --all-targets -- --test-threads=1` | 256 passed, 0 failed |
| `cargo +1.95 fmt --all -- --check` | clean |
| `cargo +1.95 clippy --all-targets -- -D warnings` | clean |
| `cargo +1.95 publish --dry-run` | packages 186 files, builds against registry `adk-* 2.0.0`, upload aborted by dry run |
| `cargo audit` | 6 advisories with fixes available, 7 unmaintained/unsound warnings |
| `cargo metadata` in a clean `git clone` without a sibling `../adk-rust` | **fails** |

All eleven `adk-*` crates depended on are published at 2.0.0. `zavora-cli` on crates.io is still 1.1.4.

## Glossary

- **Worker**: the model role that owns the conversation, tool calls, and implementation. Default `gpt-5.4-mini-2026-03-17` (`src/model_catalog.rs:5`).
- **Planner**: a bounded ADK-Rust `AgentTool` with no mutation tools, callable by the Worker for complex planning only. Default `gpt-5.6-sol` (`src/model_catalog.rs:6`).
- **Planner_Call_Budget**: a local per-process cap on Planner invocations, default 4 (`src/config.rs:561`). A guardrail, not a measure of provider-side usage.
- **Workspace**: the retained full-screen terminal surface in `src/tui.rs`.
- **Classic_Shell**: the line-oriented slash-command REPL in `src/chat.rs`, selected by `ZAVORA_CLASSIC=1`, a dumb terminal, or redirected output.
- **Automation_Surface**: the non-interactive one-shot contract in `src/headless.rs` — `text`, `json`, and `stream-json` output formats with stable exit codes.
- **Tool_Surface**: the set of ADK tools exposed to a model on a given turn, after policy filtering, wrapping, and capability routing.
- **Enforcement_Point**: the single place where a tool call is authorized before execution. In v2 this is the `ConfirmingTool` wrapper plus `filter_tools_by_policy`.
- **Capability_Pack**: a named bundle of MCP servers and skills in a category, e.g. `productivity.office` (`src/capabilities.rs`, `src/mcp_catalog.rs`).
- **Capability_State**: one of `discovered`, `configured`, `enabled`, `connected`, `authorized`. These are independent and must be reported independently.
- **Skill**: a portable capability instruction at `.agents/skills/<name>/SKILL.md`, injected through ADK-Rust's plugin runtime.
- **Plugin**: a normalized package from the Zavora, Codex, Claude, Gemini, Grok, or OpenCode ecosystems contributing skills, agents, commands, or MCP servers.
- **Specialist_Agent**: one of the bounded sub-agents in `src/agents/capability.rs` — `artifact_agent`, `developer_agent`, `research_agent`, `operations_agent`, `reviewer_agent`.
- **Phantom_Tool**: a tool named in a system prompt that is not registered in the runtime. A defect class, never acceptable.
- **Unwired_Feature**: code that compiles and may be unit-tested but has no runtime caller. A defect class, never a shipped feature.

---

## Requirements

### Requirement 1: A clean checkout builds

**User Story:** As a contributor or CI runner, I want `git clone && cargo build` to work with no sibling repositories and no undocumented setup, so that the project is actually buildable by anyone but its author.

**Current state: FAILING.** `Cargo.toml:26-36` declares all eleven ADK dependencies with `path = "../adk-rust/..."`. Cloning HEAD into a temporary directory with no sibling `adk-rust` and running `cargo metadata` fails with `failed to read /…/adk-rust/adk-browser/Cargo.toml (No such file or directory)`. `.github/workflows/ci.yml` never checks out `adk-rust`, so CI, `make release-check`, and every external contributor are broken. `cargo publish --dry-run` succeeds only because `cargo package` strips `path` and resolves the registry versions.

#### Acceptance Criteria

1. THE `Cargo.toml` manifest SHALL declare every `adk-*` dependency by registry version with no `path` key.
2. WHEN a developer wants to build against a local ADK-Rust working tree, THE repository SHALL provide an opt-in mechanism that requires no edit to a tracked file, and that mechanism SHALL be documented.
3. THE repository SHALL contain a `rust-toolchain.toml` pinning the toolchain to the version declared by `rust-version` in `Cargo.toml`.
4. WHEN `cargo metadata` is run in a fresh clone that has no sibling directories, THE command SHALL succeed.
5. WHEN `rust-version` in `Cargo.toml` changes, THE pinned toolchain and every documented MSRV statement SHALL change in the same commit.

### Requirement 2: Deliberate worker and planner routing

**User Story:** As a developer with a finite token budget, I want routine work on a cheap model and architectural reasoning on a strong model, with the split under my control and visible at all times.

**Current state: PASSING.** `zavora-cli models` prints both roles with quota pools; a mock test proves the Worker can call the Planner `AgentTool` and then hit the budget (`src/model_roles.rs:139`, `:155`). Defaults verified as `gpt-5.4-mini-2026-03-17` / `gpt-5.6-sol` against a clean config.

#### Acceptance Criteria

1. THE Worker and Planner SHALL have independent provider and model settings resolvable from CLI flags, environment variables, profile configuration, and interactive commands.
2. THE Planner SHALL be exposed as a callable ADK-Rust `AgentTool` with no mutation tools.
3. WHEN the Planner has been invoked Planner_Call_Budget times in one process, THE runtime SHALL return a structured `budget_exhausted` response instead of calling the model again.
4. WHEN the Worker role is changed at runtime, THE runtime SHALL rebuild the agent while preserving the active session service, and SHALL NOT alter the Planner configuration.
5. THE `models` command SHALL show available models, recommended roles, and shared quota pools, and SHALL NOT spend tokens.
6. THE runtime SHALL NOT spend tokens generating a greeting or banner at startup.
7. THE documented resolution precedence SHALL match the implemented precedence for both roles, including the case where only the Worker provider is configured.

### Requirement 3: Provider breadth and credential safety

**User Story:** As a developer, I want to use OpenAI, Anthropic, Gemini, DeepSeek, Groq, or a local Ollama model — optionally mixing providers across roles — without writing secrets into project files.

**Current state: PASSING with one gap.** Six providers resolve; `doctor` reports per-provider credential presence. Keys go to the OS credential vault (`src/credentials.rs:22-33`), are masked in onboarding output (`src/onboarding.rs:311`, `:348`), and are nulled before profile persistence (`:456`, `:475`). The gap is that legacy plaintext `api_key` in a profile is still accepted (`src/config.rs:81`), which interacts with Requirement 7.

#### Acceptance Criteria

1. THE runtime SHALL support OpenAI, Anthropic, Gemini, DeepSeek, Groq, and Ollama for both roles, and SHALL allow different providers per role when both credentials resolve.
2. WHEN a new credential is captured by setup or onboarding, THE runtime SHALL store it in the operating-system credential vault and SHALL NOT write it to project TOML.
3. THE runtime SHALL accept credentials from environment variables for ephemeral use.
4. WHEN a credential is displayed, logged, or included in telemetry, THE runtime SHALL mask or omit its value.
5. WHEN a legacy plaintext credential is found in profile configuration, THE runtime SHALL continue to work and SHALL warn that the value should be migrated to the vault.

### Requirement 4: A workspace that survives a long session

**User Story:** As a developer in a multi-hour session, I want the transcript, tool activity, context usage, model roles, and mode to stay visible and responsive, and I want approvals to happen in place rather than in a separate prompt.

**Current state: MOSTLY PASSING, with a durability risk.** Eleven TUI tests cover layout at common terminal sizes, unicode cursor width, word navigation, markdown rendering, parallel tool correlation by call id, palette search, slash completion, prompt history draft restore, and markdown export. The risk: `messages: Vec<Message>` (`src/tui.rs:342`) and `history: Vec<String>` (`:353`) are pushed to (`:385`, `:442`, `:1081`, `:2019`, `:3203`) with no cap, drain, or truncation, and the transcript is re-rendered from markdown on every dirty frame (`:2265`, `:2293`), so the cost of the headline feature grows with session length.

#### Acceptance Criteria

1. THE Workspace SHALL keep the conversation, tool activity, context usage, worker and planner identity, and BUILD/PLAN mode visible as the session progresses.
2. THE Workspace SHALL adapt from a wide side-by-side layout to a compact stacked layout without losing the active conversation.
3. THE Workspace SHALL render streamed Markdown and code incrementally.
4. THE Workspace SHALL provide in-place approval for consequential actions, showing the exact command or diff being approved.
5. THE Workspace SHALL bound its retained transcript, activity, and prompt-history buffers, and SHALL make the bound observable rather than silently dropping content.
6. THE Workspace redraw cost SHALL NOT grow with the number of retained messages.
7. WHEN the terminal is dumb, output is redirected, or `ZAVORA_CLASSIC=1` is set, THE runtime SHALL start the Classic_Shell instead.
8. THE Workspace and Classic_Shell SHALL expose the same runtime commands and produce consistent results for capabilities, skills, agents, MCP status, inspection, and diagnostics.
9. WHEN the developer requests cancellation, THE runtime SHALL stop the in-flight turn and SHALL terminate any process it started.

### Requirement 5: A stable automation surface

**User Story:** As a script or CI job, I want a versioned non-interactive contract with clean stdout, machine-readable output, and stable exit codes, so I can pipe a diff in and act on the result.

**Current state: PASSING.** `docs/HEADLESS.md` matches the implementation, including exit codes in `src/error.rs:43-47`. Two gaps: the sample uses a model id that is not in the catalog, and `Commands::Server` is absent from the automation match in `src/main.rs:183-192`.

#### Acceptance Criteria

1. THE Automation_Surface SHALL support `text`, `json`, and `stream-json` output formats selectable by flag and environment variable.
2. THE Automation_Surface SHALL keep stdout free of decoration, progress, and log output in `json` and `stream-json` modes.
3. THE Automation_Surface SHALL accept repeatable `--file` inputs and SHALL consume piped stdin automatically unless `--no-stdin` is given.
4. THE Automation_Surface SHALL return documented, stable exit codes distinguishing usage error, provider error, tool-approval refusal, cancellation, and internal failure.
5. WHEN a tool requires approval and no approval flag was supplied, THE Automation_Surface SHALL fail with the documented approval exit code and SHALL NOT execute the tool.
6. EVERY command that can run a model SHALL be routed through the Automation_Surface when a non-text output format is selected.
7. EVERY example in the automation documentation SHALL be executable as written.

### Requirement 6: A complete and honest tool surface

**User Story:** As a developer, I want the agent to read, search, edit, and run things competently, and I want the tools it claims to have to be the tools it actually has.

**Current state: FAILING on prompt integrity.** `src/runner.rs:37-39` advertises `file_search_agent`, `sequential_agent`, and `quality_agent` in the WORKFLOW AGENTS section, and `:47` instructs the model to use `sequential_agent` for complex work. None of the three is registered anywhere; `rg '"sequential_agent"' src/` has no match outside that prompt. Their backing modules are no-ops: `src/agents/sequential.rs:66`, `:127`, `src/agents/file_loop.rs:102`, `:118`, `src/agents/quality.rs:77` all read `Placeholder: would …`. Separately, `src/tools/lsp.rs:21` `init_manager()` has zero callers, so the LSP tool always answers `not_initialized`.

#### Acceptance Criteria

1. THE Tool_Surface SHALL include workspace-aware file reading and editing, glob and ripgrep search, shell execution, GitHub operations, todos, time, memory, release planning, and tool discovery.
2. EVERY tool named in a system prompt SHALL be registered in the runtime for that turn.
3. THE repository SHALL contain no Unwired_Feature: every module reachable from `lib.rs` SHALL either have a runtime caller, be behind a disabled feature flag, or be deleted.
4. WHEN a tool depends on lazy initialization, THE runtime SHALL perform that initialization before the tool is first offered to a model.
5. WHEN an optional feature is compiled out, THE system prompt SHALL NOT describe its tools.
6. THE file edit tool SHALL preserve line endings, reject ambiguous matches, and return a unified diff.
7. THE search tools SHALL respect `.gitignore`, exclude version-control directories, and report truncation explicitly.
8. WHEN a tool is added or removed, THE prompt, the documentation, and the registration SHALL change in the same commit.

### Requirement 7: One enforcement point for consequential actions

**User Story:** As a developer, I want a single, auditable place that decides whether a tool call runs, so that adding a tool cannot accidentally create an unguarded path to my filesystem, my shell, my network, or my secrets.

**Current state: FAILING on four counts.**

1. `src/tools/confirming.rs:345-357` auto-approves and injects `approved: true` with no prompt whenever `is_read_only_command()` passes, and `READONLY_COMMANDS` includes `cat` (`src/tools/execute_bash.rs:24`), `env` (`:46`), `printenv` (`:47`), `xxd`/`hexdump`/`strings` (`:65-67`). So `cat .env` and `printenv` run silently in the default `mcp-only` mode, while `fs_read` correctly refuses the same files (`src/tools/fs_read.rs:10-12`). Two subsystems disagree about the same secret.
2. Browser tools are appended by `tools.extend(browser_tools)` at `src/runner.rs:544`, after `filter_tools_by_policy` at `:412` and after the wrapping map has closed, so they escape both confirmation and allow/deny policy.
3. `web_fetch` is absent from the `mcp-only` guarded list at `src/runner.rs:476` and from `READ_ONLY_TOOLS` at `src/tool_policy.rs:262-269`, so it is returned unwrapped and performs unapproved network egress — while the prompt at `src/runner.rs:155` claims it requires confirmation.
4. `HookExecutor` (`src/hooks.rs:102`) is referenced only from `src/tests.rs`, so documented pre/post-tool hook configuration does nothing.

#### Acceptance Criteria

1. THE runtime SHALL apply policy filtering and confirmation wrapping at exactly one place, after the complete Tool_Surface for the turn has been assembled.
2. WHEN a tool is added to the Tool_Surface after the Enforcement_Point, THE build SHALL fail or a test SHALL fail.
3. THE runtime SHALL classify every registered tool as read-only, mutating, or network-egress, and SHALL derive auto-approval from that classification rather than from a hand-maintained name list.
4. WHEN a shell command is a candidate for read-only auto-approval, THE runtime SHALL apply the same secret and path containment rules that `fs_read` applies, and SHALL refuse auto-approval when any argument resolves to a denied path or a denied file name.
5. THE runtime SHALL NOT auto-approve any command capable of printing process environment variables.
6. WHEN a model supplies an argument that would relax a safety decision, THE Enforcement_Point SHALL ignore it.
7. WHEN a confirmation is declined, THE tool SHALL NOT execute, and the refusal SHALL be enforced at the call site rather than in the presentation layer.
8. WHEN pre-tool or post-tool hooks are configured, THE Enforcement_Point SHALL run them; IF hooks are not wired, THE documentation and configuration schema SHALL NOT offer them.
9. THE allow and deny rules SHALL apply to built-in, feature-gated, plugin-contributed, and MCP-discovered tools alike.
10. THE runtime SHALL have a regression test asserting that no registered tool bypasses the Enforcement_Point.

### Requirement 8: Shell execution is contained

**User Story:** As a developer who lets an agent run commands, I want obviously destructive or evasive commands blocked, ambiguous ones escalated to me, and the containment to survive shell tricks.

**Current state: MOSTLY PASSING, with three defects.** Twenty validators run through a pipeline (`src/tools/bash_security.rs:174-195`) with 19 passing tests covering command substitution, brace expansion, carriage returns, unicode whitespace, `jq` `system`, `/proc/environ`, comment/quote desync, and zsh-specific dangers. Defects: the spawned process has no `kill_on_drop`, so a timed-out or cancelled `sh -lc` keeps running (`src/tools/execute_bash.rs:449-453`); the retry loop retries the timeout branch (`:517-541`), so a non-idempotent command can run more than once; and the read-only early exit runs after the pipeline rather than before it, so read-only commands containing `|` or `;` escalate needlessly.

#### Acceptance Criteria

1. THE runtime SHALL validate every shell command through an ordered validator pipeline before execution.
2. THE pipeline SHALL resist quote desync, embedded newlines and carriage returns, unicode whitespace, brace expansion, command substitution, `IFS` manipulation, leading environment assignments, wrapper binaries, absolute-path aliases of blocked binaries, and case or whitespace normalization tricks.
3. WHEN a command is denied, THE runtime SHALL state which rule denied it.
4. WHEN a command times out, THE runtime SHALL terminate the process and every child it spawned before returning.
5. WHEN a turn is cancelled, THE runtime SHALL terminate any process it started.
6. THE runtime SHALL NOT retry a command that timed out.
7. THE read-only fast path SHALL be evaluated before the escalation pipeline so that safe commands are not escalated for containing shell metacharacters.
8. EVERY validator in the pipeline SHALL have at least one unit test for its allow case and one for its deny case.
9. THE shell tool SHALL NOT invoke a login shell that sources user profile files, because that reverses the environment protections the validators enforce.

### Requirement 9: Capability honesty

**User Story:** As a developer, I want to ask what this agent can do right now and get an answer derived from the live runtime, with configured, connected, and authorized reported separately, so I am never told a broken integration works.

**Current state: MOSTLY PASSING, with a consistency bug and a packaging bug.** `capabilities list` reports five categories with per-pack risk, certification, and `mcp=0/N` counts; `plugins doctor --json` distinguishes discovered JavaScript entrypoints from executable ones; `mcp protocol --json` lists remaining native gates instead of overstating compatibility; `instructions show` resolves `AGENTS.md` with scope. Bugs: `CapabilitySnapshot::load(...).unwrap_or_default()` at `src/capabilities.rs:772`, `:869`, `:942`, `:975` turns a malformed third-party manifest into a silent "0 skills · 0 plugins · 0 agents", while the prompt path at `:705-707` correctly surfaces the error — so the CLI and the prompt disagree, which `AGENTS.md` forbids. And the skills the CLI discovers are not in the repository: `.agents/` is untracked and unignored (9 `SKILL.md` files), `.gitignore:14` ignores `.skills/` (14 more), so a fresh clone finds nothing while `README.md` documents `skills validate .agents/skills/repository-development`.

#### Acceptance Criteria

1. THE runtime SHALL report Capability_State from live inspection, not from a static list.
2. THE runtime SHALL report `discovered`, `configured`, `enabled`, `connected`, and `authorized` as independent states, and SHALL NOT describe a configured or enabled integration as connected without runtime evidence.
3. WHEN capability inspection fails to parse a file, THE runtime SHALL report the failure on every surface and SHALL NOT substitute an empty result.
4. THE CLI, Classic_Shell, and Workspace SHALL produce consistent results for capabilities, skills, agents, MCP status, inspection, and diagnostics.
5. THE skills and capability content that the runtime discovers and the documentation references SHALL be present in a fresh clone.
6. THE runtime SHALL discover skills at `.agents/skills/<name>/SKILL.md` and from compatible `.zavora`, Claude, Gemini, Grok, and OpenCode roots with deterministic, documented precedence.
7. THE runtime SHALL support skill install, link, update, enable, disable, and uninstall, and SHALL inject enabled skills through ADK-Rust's plugin runtime for every model provider.
8. THE runtime SHALL normalize plugins from the Zavora, Codex, Claude, Gemini, Grok, and OpenCode ecosystems, and SHALL NOT execute package code during discovery.
9. WHEN a plugin declares a JavaScript or TypeScript entrypoint, THE runtime SHALL report it as requiring an explicit trusted runtime and SHALL NOT run it implicitly.
10. WHEN a plugin operation fails partway, THE runtime SHALL restore the previous state or report precisely what was left behind.
11. THE `AGENTS.md`, `GEMINI.md`, and `CLAUDE.md` families SHALL supply additive, scope-resolved instructions with `AGENTS.override.md` taking precedence in its directory.
12. A skill SHALL remain truthful about unavailable tooling and SHALL include verification steps for artifact creation or external writes.

### Requirement 10: MCP in both directions

**User Story:** As a developer, I want Zavora to consume tools from my MCP servers and to expose its own tool surface to other MCP clients, over stdio and HTTP, with the protocol level reported honestly.

**Current state: MOSTLY PASSING, thinly tested.** `rmcp 3.1`, stdio discovery prefers the `2026-07-28` `server/discover` lifecycle with a `2025-11-25` fallback, advertises tool-call task support, and bounds the handshake by the server timeout. `mcp catalog`/`add`/`remove` edit configuration while preserving comments. `mcp serve` exposes built-ins over stdio and returns `isError` on failure. Gaps: `src/mcp.rs` and `src/mcp_server.rs` have zero inline tests; a connect failure is reported only by `tracing::warn!` (`src/mcp.rs:372-379`), which the Workspace alternate screen swallows, so tools go silently missing; the stdio path has no reconnect or backoff while the HTTP path does; and the OAuth flow has no `state` parameter at all (`grep state src/mcp_auth.rs` returns nothing) despite building an authorization URL at `:87-91` and parsing the callback at `:219-229`.

#### Acceptance Criteria

1. THE runtime SHALL discover tools from configured stdio and Streamable HTTP MCP servers.
2. THE stdio client SHALL prefer the `2026-07-28` `server/discover` lifecycle, SHALL fall back to `2025-11-25` for compliant legacy servers, and SHALL bound the handshake by the configured server timeout.
3. THE runtime SHALL report remaining transport and authorization gates rather than claiming full protocol compatibility.
4. WHEN an MCP server fails to connect, THE runtime SHALL surface the failure on the active user surface, not only in a log sink.
5. WHEN a stdio MCP server connection drops, THE runtime SHALL retry with bounded exponential backoff and SHALL report when it gives up.
6. THE runtime SHALL expose its built-in tool surface over stdio as an MCP server, returning structured errors on failure.
7. WHEN Zavora acts as an MCP server, THE input validation of each tool SHALL still apply.
8. WHEN an MCP server requires OAuth, THE runtime SHALL perform an authorization-code flow with PKCE, SHALL include and verify a `state` parameter, SHALL bind the callback listener to loopback only, and SHALL store tokens in the credential vault.
9. WHEN a token is written to a filesystem fallback, THE runtime SHALL restrict its permissions to the owner.
10. THE MCP client and server paths SHALL have tests covering server selection, protocol negotiation and fallback, auth hinting, and tool schema stability.

### Requirement 11: Network exposure is explicit

**User Story:** As an operator, I want any listening socket to be loopback and authenticated by default, and to be told loudly when it is not.

**Current state: SAFE BY DEFAULT, UNSAFE ON REQUEST.** `server serve` defaults to `--host 127.0.0.1` (`src/cli.rs:440`). But `check_server_auth` returns `Ok(())` when no token is configured (`src/server.rs:139-141`), the token comes only from `ZAVORA_SERVER_AUTH_TOKEN` (`:418`), the comparison is not constant-time (`:153`), and `handle_server_health` has no auth check. So `--host 0.0.0.0` with no environment variable is a fully unauthenticated remote tool-execution endpoint with no warning.

#### Acceptance Criteria

1. THE server SHALL bind to loopback by default.
2. WHEN the server is asked to bind to a non-loopback address without an authentication token configured, THE runtime SHALL refuse to start or SHALL emit a prominent warning stating that tool execution is unauthenticated.
3. THE server SHALL compare authentication tokens in constant time.
4. THE health endpoint SHALL expose no configuration, profile, or workspace detail that is not safe for an unauthenticated caller.
5. WHEN a request received over the network triggers a tool requiring approval, THE runtime SHALL resolve it through the Automation_Surface contract and SHALL NOT block on the server process's stdin.

### Requirement 12: Sessions, context, and continuity

**User Story:** As a developer, I want long sessions to survive context limits, to be resumable, and to be recoverable after a bad edit, without me managing any of it by hand.

**Current state: PARTIALLY UNWIRED.** Session listing, switching, pruning, sqlite and memory backends, checkpoints, sqlite-backed memory, and Markdown export all work. But the multi-strategy compaction that v2 designed is dead code: `auto_compact()` at `src/compact.rs:508` has zero callers, and `snip_stale_tool_results()` at `:406` is reached only from inside it (`:539`); the live triggers call `compact_to_target` (`src/chat.rs:1599`) and `compact_session` (`src/tui.rs:1249`) instead.

#### Acceptance Criteria

1. THE runtime SHALL support in-memory and sqlite session backends with migration.
2. THE runtime SHALL list, show, delete, and prune sessions, and SHALL allow switching sessions inside a running Workspace or Classic_Shell.
3. WHEN the context approaches its limit, THE runtime SHALL compact automatically using the configured strategy, preferring stale-tool-result elision and file-read deduplication before summarization.
4. THE compaction strategy in use SHALL be reported to the developer when it runs.
5. THE runtime SHALL expose context usage continuously in the Workspace.
6. THE runtime SHALL support checkpoints and restoration of files it modified.
7. THE runtime SHALL persist high-signal memory across sessions and SHALL keep general knowledge out of it.
8. THE runtime SHALL export a transcript to Markdown.

### Requirement 13: Bounded delegation

**User Story:** As a developer, I want focused work delegated to a specialist when that materially improves the result, without every task being routed through a sub-agent.

**Current state: PARTIALLY FAILING.** Five Specialist_Agents exist and are registered (`src/agents/capability.rs:18`, `:24`, `:30`, `:36`, `:42`) and `agents list` shows them. `search_agent` is wired for Gemini (`src/agents/search.rs:46`, `src/runner.rs:330`). Workflow modes (single, sequential, parallel, loop) run through `src/workflow.rs`, and Ralph runs on the v2 worker/planner runtime through `src/ralph.rs`. But the three WORKFLOW AGENTS in the prompt do not exist (Requirement 6), and `.kiro/specs/ralph-orchestrator-routing/tasks.md` marks 3.1, 4.1, and 4.2 as complete for `src/agents/ralph_agent.rs`, which does not exist. `src/workflow.rs`, `src/ralph.rs`, `src/mcp.rs`, and `src/mcp_server.rs` have zero inline tests.

#### Acceptance Criteria

1. THE default agent SHALL receive a concise live capability summary.
2. THE default agent SHALL delegate bounded work to a Specialist_Agent only when that materially improves the result.
3. EVERY agent named in the delegation prompt SHALL exist and be registered under that exact name.
4. THE runtime SHALL provide single, sequential, parallel, and loop workflow modes, and SHALL make them cancellable.
5. WHEN a workflow or Ralph run is cancelled or fails partway, THE runtime SHALL report which stages completed and SHALL clean up any session it created.
6. THE Ralph pipeline SHALL run on the same v2 worker/planner runtime as chat, with a bounded iteration count and a stated termination condition.
7. THE workflow, Ralph, and delegation paths SHALL have tests covering route classification, termination, cancellation, and partial-failure aggregation.

### Requirement 14: Diagnosable by design

**User Story:** As a developer whose setup is broken, I want one command that tells me what is wrong, and telemetry that never leaks my code or my keys.

**Current state: PASSING with gaps.** `doctor` reports profile, per-provider credential presence, provider resolution, session backend, active agent, retrieval, tool confirmation, telemetry, guardrails, and MCP counts. Telemetry emits metadata only, with content capture opt-in (`src/telemetry.rs:72-79`). Gaps: `doctor` checks no language-server binaries, and LSP server stderr is discarded to `Stdio::null()` (`src/lsp/manager.rs:183`), leaving zero diagnosability when a server misbehaves.

#### Acceptance Criteria

1. THE `doctor` command SHALL validate provider credentials, session backend, profile resolution, agent selection, tool policy, telemetry, guardrails, and MCP configuration.
2. THE `doctor` command SHALL check for the external binaries the enabled features require, including language servers and `rg`.
3. THE runtime SHALL emit telemetry metadata without prompt or file content unless content capture is explicitly enabled.
4. THE runtime SHALL NOT write credentials to telemetry, logs, session storage, or checkpoints.
5. WHEN a subprocess the runtime manages writes to stderr, THE runtime SHALL capture it at debug level rather than discarding it.
6. WHEN a managed subprocess exits unexpectedly, THE runtime SHALL detect it, report it, and restart it a bounded number of times.

### Requirement 15: Documentation that matches the runtime

**User Story:** As a reader, I want every command, config key, and example in the documentation to work exactly as written.

**Current state: FAILING.** The flagship profile example is not loadable: `README.md:135` includes `auto_compact_enabled = true`, but that key lives on `RuntimeConfig` (`src/config.rs:57`), not `ProfileConfig`, which sets `deny_unknown_fields` (`src/config.rs:72`). Verified — the README block verbatim yields `[PROVIDER] invalid profile configuration in '.zavora/config.toml'`, and deleting that one line makes the identical file parse. Also: `Formula/zavora-cli.rb:4` still pins `v1.1.4.tar.gz` while `README.md:28` tells users to install from it; MSRV is stated as 1.94 in `QUICKSTART.md:5`, `CHANGELOG.md:48`, and `docs/MIGRATION_GUIDE_v2.md:7` against `Cargo.toml:5` = 1.95; `CHANGELOG.md:41` says `rmcp 2.2` against the actual 3.1; `CHANGELOG.md` has a `[2.0.0]` entry but `[Unreleased]` still holds shipped work and the file never mentions the automation surface, plugins, MCP catalog, or specialists; and `git ls-files` shows five tracked scratch files — `DOCUMENTATION_DISCREPANCIES.md`, `PROJECT_STATUS.md`, `SESSION_SUMMARY.md`, `tests/hello.py`, `tests/test_hello.py`.

#### Acceptance Criteria

1. EVERY configuration example in the documentation SHALL load without error.
2. EVERY command example in the documentation SHALL execute as written.
3. THE documented MSRV SHALL equal `rust-version` in `Cargo.toml` in every location it appears.
4. THE documented default models, providers, precedence, and budgets SHALL match the implemented defaults.
5. THE documentation SHALL describe hooks, LSP, compaction strategy, and any other capability as available only when it is wired into the runtime.
6. THE `CHANGELOG.md` release entry for a version SHALL cover every user-visible change in that version, and `[Unreleased]` SHALL hold only unreleased work.
7. THE repository SHALL NOT track ad-hoc status, session-summary, or scratch files.
8. EVERY distribution channel referenced by the documentation SHALL point at the current release.
9. WHEN a feature changes, THE documentation SHALL change in the same commit.

### Requirement 16: Trustworthy distribution and supply chain

**User Story:** As a user installing Zavora, I want every advertised channel to give me the current version, built from the tagged source, with no known-vulnerable dependencies.

**Current state: FAILING.** `cargo audit` reports six advisories with fixes available: `quinn-proto` 0.11.14 (RUSTSEC-2026-0185, 7.5 high, remote memory exhaustion), `rustls-webpki` 0.103.10 (RUSTSEC-2026-0104 reachable CRL parsing panic, RUSTSEC-2026-0098 and RUSTSEC-2026-0099 name-constraint bypasses — all in the live TLS path), `time` 0.3.45 (RUSTSEC-2026-0009, 6.8 medium), and `crossbeam-epoch` 0.9.18 (RUSTSEC-2026-0204). `cargo publish --dry-run` also warns that `spin v0.9.8` in `Cargo.lock` is yanked. `.github/workflows/ci.yml` runs `cargo check` and `cargo test` without `--all-targets` and without `--test-threads=1`, tests no feature combinations, and does not run `cargo audit`.

#### Acceptance Criteria

1. THE crates.io, npm, and Homebrew channels SHALL publish the same version from the same tag.
2. THE release pipeline SHALL verify that the tag, `Cargo.toml`, and the npm package version agree before publishing.
3. THE Homebrew formula SHALL be regenerated as part of the release, not maintained by hand.
4. CI SHALL run formatting, `check --all-targets`, `clippy --all-targets -- -D warnings`, `test --all-targets -- --test-threads=1`, the optional-feature matrix, `cargo audit`, and `cargo publish --dry-run`.
5. CI SHALL run on the pinned toolchain rather than on whatever `stable` resolves to.
6. THE release SHALL carry no dependency with a known advisory for which a fixed version exists, and SHALL carry no yanked dependency in `Cargo.lock`.
7. WHEN an advisory has no fix available, THE repository SHALL record an explicit, dated exception.
8. THE release pipeline SHALL publish checksummed binaries for linux x64/arm64 and darwin x64/arm64.

### Requirement 17: Specs are a usable release gate

**User Story:** As a maintainer, I want the spec checkboxes to describe reality, so planning is grounded in what exists rather than in what was once intended.

**Current state: FAILING in both directions.** `.kiro/specs/claude-code-capability-extraction/tasks.md` shows 0 of 143 items complete, yet roughly 104 of them are implemented and simply never ticked. `.kiro/specs/ralph-orchestrator-routing/tasks.md:37`, `:51`, `:56` mark tasks complete for `src/agents/ralph_agent.rs`, which does not exist. Six design decisions were deliberately superseded — a shared `src/jsonrpc.rs` was replaced by `rmcp`, an `McpTransport` enum by `command: Option<String>` plus `is_stdio()`, `RalphConfigBridge` by the v2-native `run_ralph` — and remain in the specs as open or completed tasks with no note.

#### Acceptance Criteria

1. A task SHALL be marked complete only when the described behavior is verifiable in the runtime.
2. WHEN an implementation deliberately diverges from a spec, THE spec SHALL be amended with the divergence and its rationale rather than left to contradict the code.
3. WHEN a spec is superseded, THE spec SHALL state which spec supersedes it.
4. THE remaining open work across all specs SHALL be reconciled into one authoritative backlog.
5. THE optional test tasks marked `- [ ]*` SHALL either be scheduled with a test framework in `[dev-dependencies]` or explicitly de-scoped.

---

## Out of scope for v2.0.0

These are recognized but deliberately deferred. They are not defects against this spec.

- Contiguous-group tool partitioning and original-order reassembly, which live upstream in `adk-agent/src/llm_agent.rs:3094-3123`.
- The `tool_search` deferral and promotion loop; per-prompt `CapabilityToolset` routing already solves the underlying problem.
- `compaction_strategy` and `tool_search_enabled` configuration keys; both features work without a switch.
- The 24 optional property tests across the four legacy specs, which need a property-testing framework in the currently empty `[dev-dependencies]`.
- An LLM-callable delegate tool allowing the orchestrator to self-delegate; `fork_sub_agent` works today only when the developer invokes it.
- Splitting `src/tui.rs` (114 KB), `src/tests.rs` (108 KB), and `src/chat.rs` (64 KB); tracked as maintainability debt, not release risk.

## Release acceptance gate

`v2.0.0` may be tagged when all of the following hold, each demonstrated by command output:

1. `cargo metadata` succeeds in a fresh clone with no sibling directories. *(R1)*
2. `cargo +<pinned> fmt --all -- --check`, `check --all-targets`, `clippy --all-targets -- -D warnings`, and `test --all-targets -- --test-threads=1` all pass, on the pinned toolchain, in CI. *(R1, R16)*
3. The optional-feature matrix compiles. *(R16)*
4. `cargo audit` reports no advisory with an available fix, and `Cargo.lock` has no yanked crate. *(R16)*
5. `cargo publish --dry-run` succeeds. *(R16)*
6. No Phantom_Tool: a test asserts every tool named in a system prompt is registered. *(R6)*
7. No Unwired_Feature: every module reachable from `lib.rs` has a runtime caller, is behind a disabled feature, or is deleted. *(R6)*
8. A test asserts no registered tool bypasses the Enforcement_Point. *(R7)*
9. A test asserts that read-only shell auto-approval refuses denied paths and denied file names, and that no auto-approved command can print the environment. *(R7)*
10. A test asserts a timed-out command's process is dead when the tool returns. *(R8)*
11. Every documented configuration and command example executes as written, checked by a script in CI. *(R15)*
12. The tag, `Cargo.toml`, the npm package, and the Homebrew formula all state the same version. *(R16)*
13. All spec checkboxes across `.kiro/specs/` reflect verified reality. *(R17)*
