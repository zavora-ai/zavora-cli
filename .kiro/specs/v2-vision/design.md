# Design Document: Zavora CLI v2 Vision

## Overview

This design closes the seventeen requirements without rewriting v2. Most of the product exists and passes its tests; what is missing is structural containment, runtime wiring, and truthfulness. The design therefore favours **removing the possibility of a class of defect** over patching its current instances.

Four structural moves carry most of the requirements:

1. **One funnel for the Tool_Surface.** Today tools are assembled, filtered, wrapped, and then *appended to again*. Two additions land after the enforcement stage. The fix is a single `ToolSurface` builder that cannot be extended after sealing, so the bypass is unrepresentable rather than merely absent. *(R6, R7)*
2. **Classification carried by the tool, not by a name list.** `adk_core::Tool` already declares `fn is_read_only()` (`../adk-rust/adk-core/src/tool.rs:108`) and `fn is_concurrency_safe()` (`:117`), and `FunctionTool` already sets them (`src/runner.rs:533-534`). But `ConfirmingTool` and `AliasedTool` forward neither, so every wrapped tool silently reports `false`, and `is_read_only_tool(name)` (`src/tool_policy.rs:273-275`) consults a hand-maintained list instead. Forwarding the flags and deriving policy from them makes classification single-sourced. *(R7.3)*
3. **The prompt is generated from the registry.** Three phantom agents survive in a hand-written prompt string (`src/runner.rs:37-39`, `:47`) because nothing connects prompt text to registration. Generating the capability and agent sections from the sealed `ToolSurface` makes a phantom impossible to write. *(R6.2, R13.3)*
4. **Secret containment is one policy, consulted by every reader.** `fs_read` refuses `.env` and `.zavora` (`src/tools/fs_read.rs:10-12`); the read-only shell fast path does not (`src/tools/confirming.rs:345-357` with `src/tools/execute_bash.rs:24`, `:46`, `:47`). Hoisting one `secret_policy` module that both consult removes the disagreement. *(R7.4, R7.5)*

Everything else is wiring (four unwired features), lifecycle (process kill, reconnect), honesty (error propagation instead of `unwrap_or_default`), or release plumbing.

## Architecture

### Dependency and build topology *(R1)*

```text
                        ┌─────────────────────────────┐
  tracked Cargo.toml ──►│ adk-* = { version = "2.0.0" }│──► crates.io
                        └─────────────────────────────┘
                                     ▲
                                     │ overridden only when present
                        ┌────────────┴────────────────┐
  untracked, opt-in ───►│ .cargo/config.toml          │──► ../adk-rust
                        │ [patch.crates-io] path = …  │
                        └─────────────────────────────┘
```

`Cargo.toml` loses every `path` key. Local ADK development is opted into by copying a tracked template to an ignored `.cargo/config.toml`, which Cargo reads automatically and which requires no edit to a tracked file. `rust-toolchain.toml` pins the toolchain so `cargo` refuses the wrong compiler with a comprehensible message instead of a bare `rustc 1.94.0 is not supported`. Both are verified by a CI job that clones the repository into a scratch directory with no siblings and runs `cargo metadata`.

### Tool surface assembly *(R6, R7)*

Current order in `resolve_runtime_tools` (`src/runner.rs:401-548`):

```text
build_builtin_tools
  → append MCP
  → filter_tools_by_policy        ◄── enforcement
  → map { wrap by decision }      ◄── enforcement
  → push tool_search              ◄── AFTER enforcement
  → extend browser tools          ◄── AFTER enforcement (escapes policy entirely)
```

Target order:

```text
ToolSurface::new()
  .add_builtins()
  .add_feature_gated()      // browser, sandbox, rag, web_fetch, lsp
  .add_plugin_contributed()
  .add_mcp(discovered)
  .add_discovery_tool()     // tool_search, over the assembled set
  .seal(cfg)                ◄── the only Enforcement_Point
  → ResolvedRuntimeTools    // opaque; no push, extend, or insert
```

`seal` performs, in order: classify → deny-filter → decide → wrap → freeze. It returns `ResolvedRuntimeTools` whose tool vector is private with no mutating accessor, so a future `tools.extend(...)` fails to compile. `tool_search` moves inside the builder so it searches the sealed set rather than a pre-enforcement snapshot.

### Enforcement decision *(R7)*

```text
                    ┌──────────────┐
  tool ────────────►│ ToolClass    │  ReadOnly | Mutating | NetworkEgress
                    └──────┬───────┘  derived from Tool::is_read_only()
                           │          + explicit egress registry
                    ┌──────▼───────────────┐
                    │ PermissionRules      │  Deny → drop
                    │ .evaluate(name, arg) │  Ask  → wrap
                    └──────┬───────────────┘  Allow → wrap_display_only
                           │
                    ┌──────▼───────────────┐
                    │ ToolConfirmationMode │  Always | McpOnly | Never
                    └──────┬───────────────┘
                    ┌──────▼───────────────┐
                    │ argument scrubbing   │  strip model-supplied
                    │                      │  approval/danger keys
                    └──────┬───────────────┘
                    ┌──────▼───────────────┐
                    │ hook stage           │  PreTool → execute → PostTool
                    └──────────────────────┘
```

Two decisions change materially. `NetworkEgress` is never auto-approved regardless of mode, which fixes `web_fetch` without a name list. And argument scrubbing runs inside `ConfirmingTool::execute`, so a model-supplied `allow_dangerous` (`src/tools/execute_bash.rs:214`) cannot relax a verdict the enforcement layer already made.

### Read-only shell fast path *(R7.4, R7.5, R8.7)*

```text
command ──► READONLY_COMMANDS lookup
              │ miss ──────────────────────────► validator pipeline ──► Ask/Deny
              │ hit
              ▼
            secret_policy::scan_arguments(command)
              │ any denied path or denied file name ──► validator pipeline (no auto-approve)
              │ command can print environment ───────► never auto-approve
              ▼
            auto-approve
```

Three consequences: the fast path runs *before* the escalation pipeline so `ls | wc -l` stops escalating; `env` and `printenv` leave `READONLY_COMMANDS` entirely; and `cat`, `xxd`, `hexdump`, `strings`, `head`, `tail` keep their fast path but lose it for any argument `fs_read` would refuse.

## Components and Interfaces

### 1. `.cargo/config.toml.local-adk` template and `rust-toolchain.toml` *(R1)*

Tracked template plus a `.gitignore` entry for `.cargo/config.toml`. `rust-toolchain.toml` declares `channel = "1.95.0"` with `components = ["rustfmt", "clippy"]`. A `make local-adk` target copies the template; `make unlink-adk` removes it. Documented in `README.md` under Development.

### 2. `src/tools/secret_policy.rs` (new) *(R7.4, R7.5)*

Hoists the containment lists currently private to `fs_read`:

```rust
pub const DENIED_SEGMENTS: &[&str];      // from fs_read.rs:10
pub const DENIED_FILE_NAMES: &[&str];    // from fs_read.rs:11-12

pub fn is_denied_path(path: &Path) -> bool;
pub fn scan_command_arguments(command: &str) -> Vec<PathBuf>; // shlex-split, path-like args
pub fn command_reads_environment(argv0: &str) -> bool;
```

`fs_read` is refactored to call `is_denied_path` so the two paths cannot drift. `scan_command_arguments` uses the existing `shlex` dependency and resolves each candidate against the workspace root before comparison, so `cat ./.env` and `cat "$PWD/.env"` are both caught by path resolution rather than string matching.

### 3. `ToolClass` and wrapper forwarding *(R7.1, R7.3)*

```rust
pub enum ToolClass { ReadOnly, Mutating, NetworkEgress }

pub fn classify(tool: &Arc<dyn Tool>) -> ToolClass;
```

`classify` reads `Tool::is_read_only()` first and consults an explicit egress registry for tools that touch the network (`web_fetch`, `github_ops`, browser tools, MCP-discovered tools). `is_read_only_tool(name: &str)` and `READ_ONLY_TOOLS` are deleted. `ConfirmingTool` and `AliasedTool` gain forwarding implementations of `is_read_only` and `is_concurrency_safe`, which also re-enables ADK's concurrent tool execution — currently inert because every wrapped tool reports `false`.

### 4. `ToolSurface` builder and sealed `ResolvedRuntimeTools` *(R6.2, R6.3, R7.1, R7.2)*

```rust
pub struct ToolSurface { tools: Vec<Arc<dyn Tool>>, mcp_names: BTreeSet<String> }

impl ToolSurface {
    pub fn new() -> Self;
    pub fn add_builtins(self) -> Self;
    pub fn add_feature_gated(self) -> impl Future<Output = Self>;
    pub fn add_plugin_contributed(self, snapshot: &CapabilitySnapshot) -> Self;
    pub fn add_mcp(self, tools: Vec<Arc<dyn Tool>>) -> Self;
    pub fn add_discovery_tool(self, threshold: usize) -> Self;
    pub fn seal(self, cfg: &RuntimeConfig) -> ResolvedRuntimeTools;
}

pub struct ResolvedRuntimeTools {   // fields private
    pub fn tools(&self) -> &[Arc<dyn Tool>];
    pub fn names(&self) -> BTreeSet<&str>;
    pub fn mcp_tool_names(&self) -> &BTreeSet<String>;
}
```

The browser block moves from `src/runner.rs:539-545` into `add_feature_gated`. `tool_search` moves from `:517-536` into `add_discovery_tool`.

### 5. `PromptSurface` *(R6.2, R6.5, R6.8, R13.3)*

```rust
pub struct PromptSurface<'a> { registered: BTreeSet<&'a str>, agents: BTreeSet<&'a str> }

impl<'a> PromptSurface<'a> {
    pub fn from(tools: &'a ResolvedRuntimeTools, agents: &'a [Arc<dyn Agent>]) -> Self;
    pub fn render_capability_section(&self) -> String;
    pub fn render_agent_section(&self) -> String;
    pub fn assert_no_phantoms(&self, prompt: &str) -> Result<(), Vec<String>>;
}
```

The prose parts of `ORCHESTRATOR_INSTRUCTION` stay hand-written; the enumerations become generated. `assert_no_phantoms` extracts every `- <name>:` bullet from the composed prompt and reports any name absent from the registry — driving the release-gate test. The WORKFLOW AGENTS block (`src/runner.rs:36-39`) and the `sequential_agent` rule (`:47`) are deleted, and `src/agents/{sequential,file_loop,quality}.rs` are removed along with their `pub mod` lines in `src/agents/mod.rs:15`, `:18`, `:20`.

### 6. Process lifecycle in `execute_bash` *(R4.9, R8.4, R8.5, R8.6, R8.9)*

The spawn at `src/tools/execute_bash.rs:449-453` gains `.kill_on_drop(true)` and moves to explicit child handling so the timeout path kills before returning:

```rust
let mut child = Command::new("sh").arg("-c").arg(cmd)
    .kill_on_drop(true).process_group(0).spawn()?;
match timeout(dur, child.wait_with_output()).await {
    Ok(out) => out,
    Err(_) => { kill_process_group(&child)?; return timed_out(); }
}
```

Three further changes: `-lc` becomes `-c` so a login shell cannot re-source the PATH and `LD_PRELOAD` protections asserted at `src/tools/bash_security.rs:306-319`; `process_group(0)` plus a group kill reaps children the command spawned; and the retry loop (`:517-541`) matches on the error variant and never retries a timeout. Cancellation reuses the same kill path so Esc in the Workspace (`src/tui.rs:864-871`) leaves no orphan.

### 7. Hook stage in `ConfirmingTool` *(R7.8)*

`HookExecutor` (`src/hooks.rs:102`, currently referenced only from `src/tests.rs`) is threaded through `ConfirmingTool` as an `Option<Arc<HookExecutor>>` populated during `seal`. `PreTool` runs after the approval decision and before `execute`; `PostTool` runs after. A `PreTool` hook returning a block decision aborts without executing. If this task is deferred, the alternative is removal of hook configuration from `ProfileConfig` and from the documentation — the requirement forbids offering unwired configuration, not having hooks.

### 8. LSP lifecycle *(R6.4, R14.5, R14.6)*

`crate::tools::lsp::init_manager()` (`src/tools/lsp.rs:21`, zero callers) is called from `add_feature_gated` before the `lsp` tool is registered, and `shutdown()` is called from the Workspace, Classic_Shell, and Automation_Surface exit paths. `src/lsp/manager.rs:183` replaces `Stdio::null()` with a piped stderr drained into `tracing::debug!`. The existing but never-incremented `restart_count` (`src/lsp/manager.rs:81`) is wired to a bounded restart on unexpected exit. `doctor` gains a language-server and `rg` presence check.

### 9. Compaction strategy *(R12.3, R12.4)*

`auto_compact()` (`src/compact.rs:508`, zero callers) becomes the entry point for the automatic trigger, replacing the direct `compact_to_target` call at `src/chat.rs:1599` and `compact_session` at `src/tui.rs:1249`. Its internal order — `snip_stale_tool_results` and file-read deduplication first, summarization as fallback — becomes the reported behaviour, and the strategy actually used is emitted as a system event so the developer sees which one ran.

### 10. Workspace durability *(R4.5, R4.6)*

```rust
const MAX_RETAINED_MESSAGES: usize = 500;
const MAX_PROMPT_HISTORY: usize = 200;

struct Message { /* … */ rendered: OnceCell<Vec<Line<'static>>> }
```

`messages` (`src/tui.rs:342`) and `history` (`:353`) gain bounded pushes that drain from the front and insert a single retained elision marker (`… N earlier messages elided`) so truncation is visible rather than silent. Each `Message` caches its rendered lines, invalidated only when its own text changes, so `draw_transcript` (`:2265`, `:2293`) stops re-parsing Markdown for the whole transcript on every dirty frame.

### 11. Capability snapshot error propagation *(R9.3, R9.4)*

The four `CapabilitySnapshot::load(...).unwrap_or_default()` call sites (`src/capabilities.rs:772`, `:869`, `:942`, `:975`) and `load_state().unwrap_or_default()` (`:766`) return `Result` to their callers and render the parse failure, matching the prompt path that already does so (`:705-707`). A malformed third-party manifest must never present as "0 skills · 0 plugins · 0 agents".

### 12. MCP robustness *(R10.4, R10.5, R10.8, R10.9)*

Connect failures currently reported only by `tracing::warn!` (`src/mcp.rs:372-379`) are collected into a `Vec<McpConnectFailure>` returned alongside the tools and rendered by whichever surface is active. The stdio path (`:286-320`) gains the bounded backoff the HTTP path already has via `RefreshConfig` (`:261-263`). `src/mcp_auth.rs` gains a `state` parameter — generated with the existing `rand` dependency, included in the authorization URL (`:87-91`), and compared against the callback query (`:219-229`) with rejection on mismatch. The filesystem token fallback (`:269-274`) is written with mode `0o600`.

### 13. Server exposure guard *(R11)*

`check_server_auth` (`src/server.rs:135-160`) keeps its optional-token behaviour for loopback but `serve` refuses a non-loopback bind when `auth_token` is `None`, overridable only by an explicit `--insecure-no-auth` flag that logs a warning on every request. Token comparison uses a constant-time equality. `handle_server_health` (`:163-171`) drops `profile` and `app_name` unless authenticated. `Commands::Server` is added to the automation match in `src/main.rs:183-192` so a network-triggered tool never blocks on the server process's stdin.

### 14. Content packaging and documentation harness *(R9.5, R15)*

`.agents/` is committed — 9 `SKILL.md` files the runtime discovers and `README.md` references. `.skills/` is resolved one way: if it is product surface it is tracked and `.gitignore:14` is removed; if it is user-local scratch, the documentation says so and stops implying a fresh clone has skills. The `test/` entry at `.gitignore:12` is corrected or removed. Five tracked scratch files are deleted: `DOCUMENTATION_DISCREPANCIES.md`, `PROJECT_STATUS.md`, `SESSION_SUMMARY.md`, `tests/hello.py`, `tests/test_hello.py`.

A new `scripts/check_docs.sh` extracts fenced blocks from `README.md`, `QUICKSTART.md`, and `docs/*.md`, writes each `toml` block tagged as profile configuration to a scratch `.zavora/config.toml` and asserts `profiles show` succeeds, and asserts every `zavora-cli <subcommand>` in a `bash` block resolves against `--help` output. This is what makes R15.1 and R15.2 enforceable rather than aspirational; it is wired into `make ci`.

## Data Models

### ToolClass

```rust
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ToolClass {
    ReadOnly,        // auto-approvable; display-only wrap
    Mutating,        // filesystem, git, or process mutation
    NetworkEgress,   // leaves the machine; never auto-approved
}
```

### ResolvedRuntimeTools (sealed)

| Field | Visibility | Purpose |
|---|---|---|
| `tools: Vec<Arc<dyn Tool>>` | private | wrapped, frozen tool set |
| `classes: BTreeMap<String, ToolClass>` | private | classification recorded at seal time |
| `mcp_tool_names: BTreeSet<String>` | private | provenance for confirmation mode |
| `connect_failures: Vec<McpConnectFailure>` | private | surfaced by the active UI |

Accessors are read-only. No `push`, `extend`, `insert`, or `&mut` accessor exists.

### McpConnectFailure

```rust
pub struct McpConnectFailure {
    pub server: String,
    pub transport: &'static str,   // "stdio" | "http"
    pub error: String,
    pub retryable: bool,
}
```

### SecretScanResult

```rust
pub struct SecretScanResult {
    pub denied_paths: Vec<PathBuf>,
    pub reads_environment: bool,
}
```

### CompactionOutcome

```rust
pub struct CompactionOutcome {
    pub strategy: &'static str,   // "snip" | "dedup" | "summarize"
    pub events_before: usize,
    pub events_after: usize,
}
```

### Message (Workspace)

| Field | Purpose |
|---|---|
| `role`, `text` | existing content |
| `rendered: OnceCell<Vec<Line<'static>>>` | cached render, invalidated on text change |
| `elided: bool` | marks the single retained elision marker |

## Correctness Properties

These are the properties the release gate tests assert. Each names the requirement it validates.

**Property 1: Clean-checkout resolvability.** For a clone of `HEAD` into any directory with no sibling `adk-rust`, `cargo metadata` succeeds. *(R1.1, R1.4)*

**Property 2: Surface sealing.** No code path can add a tool to `ResolvedRuntimeTools` after `seal`. Enforced by visibility, asserted by a test that the tool count after `seal` equals the count the enforcement stage observed. *(R7.1, R7.2)*

**Property 3: No phantom tools.** For every system prompt the runtime can compose, every tool or agent name enumerated in it is present in the sealed registry. *(R6.2, R13.3)*

**Property 4: No unwired modules.** Every module reachable from `lib.rs` has at least one non-test caller, or is behind a feature flag that is off by default. *(R6.3)*

**Property 5: Classification totality.** Every tool in the sealed surface has exactly one `ToolClass`, and no tool classified `NetworkEgress` is ever returned unwrapped in any `ToolConfirmationMode`. *(R7.3, R7.6)*

**Property 6: Secret containment agreement.** For every path `fs_read` refuses, a shell command taking that path as an argument is not auto-approved. No command in the read-only fast path can print process environment variables. *(R7.4, R7.5)*

**Property 7: Argument scrubbing.** For any model-supplied argument set, the enforcement decision is identical to the decision for that argument set with approval and danger keys removed. *(R7.6)*

**Property 8: Process reaping.** After a shell tool returns a timeout or a cancellation, no process it started is alive, including grandchildren. *(R8.4, R8.5, R4.9)*

**Property 9: No timeout retry.** A command that times out is executed exactly once. *(R8.6)*

**Property 10: Fast-path precedence.** A command in `READONLY_COMMANDS` with clean arguments is auto-approved regardless of shell metacharacters that would otherwise escalate it. *(R8.7)*

**Property 11: Failure visibility.** Every capability, plugin, and MCP failure that changes what the runtime can do is rendered on the active surface, and no surface substitutes an empty result for a parse error. *(R9.3, R9.4, R10.4)*

**Property 12: Documentation executability.** Every fenced configuration block in the documentation loads, and every `zavora-cli` invocation in the documentation resolves to a real subcommand and flag set. *(R15.1, R15.2)*

**Property 13: Buffer bounding with visible elision.** Retained Workspace buffers never exceed their cap, and any elision is represented by exactly one visible marker. *(R4.5)*

**Property 14: Version agreement.** The git tag, `Cargo.toml`, `npm/zavora-cli/package.json`, and `Formula/zavora-cli.rb` state the same version. *(R16.1, R16.2)*

## Error Handling

| Condition | Behaviour | Requirement |
|---|---|---|
| Sibling `adk-rust` absent | Build succeeds from registry; local override only when the ignored config file exists | R1.2 |
| Wrong toolchain | `rust-toolchain.toml` selects the pinned one; if unavailable, Cargo reports the required version | R1.3 |
| Read-only shell command with a denied argument | Falls through to the validator pipeline and escalates to Ask; never auto-approved | R7.4 |
| Command that prints the environment | Never auto-approved, in any confirmation mode | R7.5 |
| Model supplies an approval or danger key | Key stripped before the decision; decision unchanged | R7.6 |
| Confirmation declined | Tool does not execute; refusal enforced at the call site | R7.7 |
| Shell timeout | Process group killed, single structured timeout result, no retry | R8.4, R8.6 |
| Turn cancelled | Same kill path; Workspace reports cancellation only after the process is dead | R4.9, R8.5 |
| Capability or plugin manifest malformed | Parse error rendered on every surface; never an empty snapshot | R9.3 |
| Plugin operation fails mid-way | State restored, or the exact residue reported with the path | R9.10 |
| MCP server unreachable | Failure surfaced on the active surface; stdio retried with bounded backoff, then reported as given up | R10.4, R10.5 |
| OAuth `state` mismatch | Callback rejected, no token exchange | R10.8 |
| Non-loopback bind without a token | Refuse to start unless `--insecure-no-auth`, which warns per request | R11.2 |
| LSP server exits unexpectedly | Detected, reported, restarted a bounded number of times, stderr at debug | R14.5, R14.6 |
| Advisory with no fix available | Dated exception recorded in the repository | R16.7 |

## Testing Strategy

The suite stays deterministic and uses ADK-Rust mock models; no test calls a paid model. `[dev-dependencies]` remains empty unless a property-testing framework is adopted, which is out of scope, so the properties above are asserted as targeted unit and integration tests rather than generated ones.

### Release-gate tests (new, each maps to a Property)

| Test | Location | Property |
|---|---|---|
| clean-clone resolvability | CI job, `scripts/check_clean_clone.sh` | P1 |
| sealed surface count invariance | `src/tests.rs` | P2 |
| prompt names ⊆ registry | `src/tests.rs` | P3 |
| module caller census | `scripts/check_wiring.sh` in `make ci` | P4 |
| classification totality; egress never unwrapped | `src/tests.rs` | P5 |
| `fs_read` deny list ≡ shell fast-path deny list | `src/tools/secret_policy.rs` tests | P6 |
| `env`/`printenv` never auto-approved | `src/tools/execute_bash.rs` tests | P6 |
| scrubbed vs unscrubbed decision equality | `src/tools/confirming.rs` tests | P7 |
| timeout leaves no live process | `src/tools/execute_bash.rs` tests | P8 |
| timeout executes once | `src/tools/execute_bash.rs` tests | P9 |
| fast path precedes escalation | `src/tools/execute_bash.rs` tests | P10 |
| snapshot parse error surfaces | `src/capabilities.rs` tests | P11 |
| documentation examples execute | `scripts/check_docs.sh` in `make ci` | P12 |
| buffer cap and single elision marker | `src/tui.rs` tests | P13 |
| version agreement | `make version-check`, extended to the formula | P14 |

### Coverage for the four untested modules *(R10.10, R13.7)*

`src/workflow.rs`, `src/ralph.rs`, `src/mcp.rs`, and `src/mcp_server.rs` have zero inline tests today. Ranked by blast radius:

1. `select_mcp_servers` enabled/disabled/aliased filtering (`src/mcp.rs:54`).
2. Protocol negotiation and `2026-07-28` → `2025-11-25` fallback with a bounded handshake (`src/mcp.rs:239`).
3. `resolve_mcp_auth` and auth hinting (`src/mcp.rs:76`, `:184`).
4. `classify_workflow_route` (`src/workflow.rs:59`).
5. `run_ralph` loop termination and cancellation reporting (`src/ralph.rs:21`).
6. `mcp_server` tool schema stability and `isError` on failure (`src/mcp_server.rs:69-104`).

### Validator coverage completion *(R8.8)*

Nineteen validator tests exist and pass. Missing allow/deny pairs: `validate_redirections`, `validate_obfuscated_flags`, `validate_ifs_injection`, `validate_mid_word_hash`, `validate_malformed_token_injection`, and background `&`.

### CI gates *(R16.4, R16.5)*

`make ci` becomes: `fmt-check`, `check --all-targets`, `lint`, `test --all-targets -- --test-threads=1`, feature matrix, `check_wiring.sh`, `check_docs.sh`, `quality-gate`, `security-check`, `cargo audit`. `make release-check` adds `check_clean_clone.sh`, `cargo publish --dry-run`, and `version-check`. The workflow switches from `dtolnay/rust-toolchain@stable` to the pinned toolchain so a future stable release cannot silently change the compiler under the release.
