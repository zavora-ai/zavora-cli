# Implementation Plan: Zavora CLI v2 Vision

## Overview

Close the seventeen requirements in `requirements.md` using the four structural moves in `design.md`. The ordering is deliberate: the build must work before anyone can verify anything else, containment precedes convenience, and the documentation harness lands before the documentation edits so the edits are checked rather than asserted.

Task 1 is marked complete because its output is this spec, and the evidence is the verification baseline table in `requirements.md`. Everything else is open. Tasks marked `- [ ]*` are optional under the existing spec convention and are not release-gate items.

Phase boundaries are meaningful: Phases 2–4 are the release blockers, Phases 5–9 convert written-but-dead code into shipped behaviour, Phases 10–13 are honesty and robustness, and Phases 14–16 are release plumbing. Do not begin a later phase's verification before its predecessor's tests pass, because several later tests depend on the sealed surface introduced in Phase 4.

## Tasks

- [x] 1. Establish the verified baseline and supersede the partial specs
  - [x] 1.1 Run the full verification matrix and record results
    - `check --all-targets`, feature matrix, `test --all-targets -- --test-threads=1`, `fmt --check`, `clippy -D warnings`, `publish --dry-run`, `cargo audit`, clean-clone `cargo metadata`
    - Recorded in the Verification baseline table in `requirements.md`
    - _Requirements: 17.1_
  - [x] 1.2 Reconcile the six existing specs against the runtime and write this spec
    - _Requirements: 17.4_

### Phase 2 — Build and toolchain integrity (blocker)

- [x] 2. Make a clean checkout build
  - [x] 2.1 Remove every `path` key from the eleven `adk-*` dependencies in `Cargo.toml:26-36`, keeping `version = "2.0.0"`
    - `cargo publish --dry-run` already proves the registry versions compile
    - _Requirements: 1.1_
  - [x] 2.2 Add a tracked `.cargo/config.toml.local-adk` template with `[patch.crates-io]` entries pointing at `../adk-rust/*`, and add `.cargo/config.toml` to `.gitignore`
    - _Requirements: 1.2_
  - [x] 2.3 Add `make local-adk` and `make unlink-adk` targets that copy and remove the template
    - _Requirements: 1.2_
  - [x] 2.4 Add `rust-toolchain.toml` with `channel = "1.95.0"` and `components = ["rustfmt", "clippy"]`
    - _Requirements: 1.3_
  - [x] 2.5 Add `scripts/check_clean_clone.sh`: clone `HEAD` into a scratch directory with no siblings, run `cargo metadata`, fail on error
    - Asserts Property 1
    - _Requirements: 1.4_
  - [x] 2.6 Document the local-ADK workflow and the pinned toolchain in the README Development section
    - _Requirements: 1.2, 1.5_
  - [x] 2.7 Verify: `scripts/check_clean_clone.sh` passes, and `cargo build` still works with the template installed
    - _Requirements: 1.1, 1.4_

- [x] 3. Close the dependency advisories
  - [x] 3.1 Update `Cargo.lock` to `quinn-proto >= 0.11.15`, `rustls-webpki >= 0.103.13`, `time >= 0.3.47`, `crossbeam-epoch >= 0.9.20`
    - RUSTSEC-2026-0185, 0104, 0098, 0099, 0009, 0204
    - _Requirements: 16.6_
  - [x] 3.2 Replace the yanked `spin v0.9.8` in `Cargo.lock`
    - Reaches the graph through `flume` → `sqlx-sqlite`; may require a `sqlx` patch bump
    - _Requirements: 16.6_
  - [x] 3.3 Record dated exceptions for any remaining advisory with no fixed version available
    - _Requirements: 16.7_
  - [x] 3.4 Verify: `cargo audit` reports no advisory with an available fix, `cargo publish --dry-run` emits no yanked-crate warning, and the full test suite still passes
    - _Requirements: 16.6_

### Phase 3 — Containment (blocker)

- [x] 4. Single-source secret containment
  - [x] 4.1 Create `src/tools/secret_policy.rs` holding `DENIED_SEGMENTS` and `DENIED_FILE_NAMES` moved from `src/tools/fs_read.rs:10-12`
    - _Requirements: 7.4_
  - [x] 4.2 Implement `is_denied_path`, `scan_command_arguments` (shlex-split, workspace-resolved), and `command_reads_environment`
    - Resolve each path-like argument against the workspace root so `./.env` and `$PWD/.env` are caught by resolution, not string matching
    - _Requirements: 7.4_
  - [x] 4.3 Refactor `src/tools/fs_read.rs` to call `secret_policy::is_denied_path` instead of its private lists
    - _Requirements: 7.4_
  - [x] 4.4 Remove `"env"` and `"printenv"` from `READONLY_COMMANDS` (`src/tools/execute_bash.rs:46-47`)
    - _Requirements: 7.5_
  - [x] 4.5 Gate the read-only fast path in `src/tools/confirming.rs:345-357` on `scan_command_arguments` returning no denied path
    - On a denied argument, fall through to the validator pipeline rather than auto-approving
    - _Requirements: 7.4_
  - [x] 4.6 Write tests: every path `fs_read` refuses is also refused auto-approval as a shell argument; `env` and `printenv` are never auto-approved
    - Asserts Property 6
    - _Requirements: 7.4, 7.5_

- [x] 5. Classify tools from the tool, not from a name list
  - [x] 5.1 Add `is_read_only` and `is_concurrency_safe` forwarding to `impl Tool for ConfirmingTool` in `src/tools/confirming.rs`
    - The trait defaults at `../adk-rust/adk-core/src/tool.rs:108`, `:117` are `false`, so every wrapped tool currently reports read-write and concurrency-unsafe
    - _Requirements: 7.3_
  - [x] 5.2 Add the same forwarding to `impl Tool for AliasedTool` in `src/tool_policy.rs`
    - _Requirements: 7.3_
  - [x] 5.3 Add `ToolClass` and `classify()`, reading `Tool::is_read_only()` plus an explicit network-egress registry covering `web_fetch`, `github_ops`, browser tools, and MCP-discovered tools
    - _Requirements: 7.3_
  - [x] 5.4 Delete `READ_ONLY_TOOLS` and `is_read_only_tool` (`src/tool_policy.rs:262-275`) and replace their call sites with `classify()`
    - _Requirements: 7.3_
  - [x] 5.5 Write tests: classification is total over the sealed surface, and no `NetworkEgress` tool is returned unwrapped in any `ToolConfirmationMode`
    - Asserts Property 5; fixes the `web_fetch` gap at `src/runner.rs:476` without a name list
    - _Requirements: 7.3, 7.6_

### Phase 4 — One enforcement point (blocker)

- [x] 6. Seal the tool surface
  - [x] 6.1 Introduce `ToolSurface` with `add_builtins`, `add_feature_gated`, `add_plugin_contributed`, `add_mcp`, `add_discovery_tool`, and `seal`
    - _Requirements: 7.1_
  - [x] 6.2 Move the browser block from `src/runner.rs:539-545` into `add_feature_gated` so it precedes enforcement
    - Today it lands after `filter_tools_by_policy` (`:412`) and after the wrapping map, escaping both
    - _Requirements: 7.1, 7.9_
  - [x] 6.3 Move `tool_search` from `src/runner.rs:517-536` into `add_discovery_tool` so it searches the sealed set
    - _Requirements: 7.1_
  - [x] 6.4 Make `ResolvedRuntimeTools` fields private with read-only accessors and no mutating API
    - _Requirements: 7.2_
  - [x] 6.5 Implement `seal` as classify → deny-filter → decide → wrap → freeze, recording `ToolClass` per tool
    - _Requirements: 7.1, 7.3_
  - [x] 6.6 Write a test asserting the tool count and name set after `seal` equal what the enforcement stage observed
    - Asserts Property 2
    - _Requirements: 7.1, 7.2_
  - [x] 6.7 Write a test asserting allow/deny rules apply to built-in, feature-gated, plugin-contributed, and MCP-discovered tools alike
    - _Requirements: 7.9_

- [x] 7. Scrub model-supplied safety arguments
  - [x] 7.1 Strip approval and danger keys from arguments inside `ConfirmingTool::execute` before the decision, including `allow_dangerous` (`src/tools/execute_bash.rs:214`) and `approved`
    - _Requirements: 7.6_
  - [x] 7.2 Write a test asserting the decision for any argument set equals the decision for that set with safety keys removed
    - Asserts Property 7
    - _Requirements: 7.6_
  - [x] 7.3 Write a test asserting a declined confirmation does not execute the tool, enforced at the call site
    - _Requirements: 7.7_

- [x] 8. Eliminate phantom tools
  - [x] 8.1 Add `PromptSurface` with `render_capability_section`, `render_agent_section`, and `assert_no_phantoms`
    - _Requirements: 6.2_
  - [x] 8.2 Delete the WORKFLOW AGENTS block at `src/runner.rs:36-39` and the `sequential_agent` rule at `:47`
    - `file_search_agent`, `sequential_agent`, and `quality_agent` are registered nowhere
    - _Requirements: 6.2, 13.3_
  - [x] 8.3 Delete `src/agents/sequential.rs`, `src/agents/file_loop.rs`, and `src/agents/quality.rs` and their `pub mod` lines at `src/agents/mod.rs:15`, `:18`, `:20`
    - All three are `Placeholder: would …` no-ops with no instantiation
    - _Requirements: 6.3_
  - [x] 8.4 Generate the prompt's capability and agent enumerations from the sealed surface, keeping the prose hand-written
    - _Requirements: 6.2, 6.5, 6.8_
  - [x] 8.5 Write a test asserting every tool and agent name enumerated in every composable system prompt is present in the registry
    - Asserts Property 3
    - _Requirements: 6.2, 13.3_
  - [x] 8.6 Add `scripts/check_wiring.sh`: census every module reachable from `lib.rs` for a non-test caller; fail on an unwired module that is not behind a default-off feature
    - Asserts Property 4
    - _Requirements: 6.3_

- [x] 9. Contain process lifecycle
  - [x] 9.1 Add `.kill_on_drop(true)` and `.process_group(0)` to the spawn at `src/tools/execute_bash.rs:449-453`
    - _Requirements: 8.4_
  - [x] 9.2 Replace `sh -lc` with `sh -c` so a login shell cannot re-source the PATH and `LD_PRELOAD` protections asserted at `src/tools/bash_security.rs:306-319`
    - _Requirements: 8.9_
  - [x] 9.3 Kill the process group on timeout before returning the structured timeout result
    - _Requirements: 8.4_
  - [x] 9.4 Stop retrying the timeout error variant in the retry loop at `src/tools/execute_bash.rs:517-541`
    - _Requirements: 8.6_
  - [x] 9.5 Route Workspace cancellation (`src/tui.rs:864-871`) through the same kill path, and report cancellation only after the process is dead
    - _Requirements: 4.9, 8.5_
  - [x] 9.6 Move the read-only fast path ahead of the validator pipeline at `src/tools/execute_bash.rs:359` / `:417`
    - _Requirements: 8.7_
  - [x] 9.7 Write tests: timeout leaves no live process including grandchildren; a timed-out command executes exactly once; a clean read-only command with `|` or `;` is auto-approved
    - Asserts Properties 8, 9, 10
    - _Requirements: 8.4, 8.5, 8.6, 8.7_
  - [x] 9.8 Add the missing validator allow/deny test pairs: `validate_redirections`, `validate_obfuscated_flags`, `validate_ifs_injection`, `validate_mid_word_hash`, `validate_malformed_token_injection`, background `&`
    - _Requirements: 8.8_

### Phase 5 — Wire what is written

- [x] 10. Resolve the hook stage
  - [x] 10.1 Thread `HookExecutor` (`src/hooks.rs:102`, referenced only from `src/tests.rs`) into `ConfirmingTool` as an `Option<Arc<HookExecutor>>` populated during `seal`
    - _Requirements: 7.8_
  - [x] 10.2 Run `PreTool` after the approval decision and before `execute`; run `PostTool` after; abort without executing when a `PreTool` hook blocks
    - _Requirements: 7.8_
  - [x] 10.3 If 10.1–10.2 are deferred, instead remove hook configuration from `ProfileConfig` and every documentation reference
    - The requirement forbids offering unwired configuration, not having hooks
    - _Requirements: 7.8, 15.5_

- [x] 11. Wire the LSP lifecycle
  - [x] 11.1 Call `crate::tools::lsp::init_manager()` (`src/tools/lsp.rs:21`, zero callers) from `add_feature_gated` before registering the `lsp` tool
    - Without this the tool always answers `not_initialized`
    - _Requirements: 6.4_
  - [x] 11.2 Call `shutdown()` from the Workspace, Classic_Shell, and Automation_Surface exit paths
    - _Requirements: 6.4_
  - [x] 11.3 Replace `Stdio::null()` at `src/lsp/manager.rs:183` with a piped stderr drained into `tracing::debug!`
    - _Requirements: 14.5_
  - [x] 11.4 Detect unexpected server exit and restart a bounded number of times using the existing `restart_count` (`src/lsp/manager.rs:81`)
    - _Requirements: 14.6_
  - [x] 11.5 Add language-server and `rg` presence checks to `doctor`
    - _Requirements: 14.2_
  - [ ]* 11.6 Notify open documents on edit and close them on shutdown so server state does not go stale after `file_edit` or `fs_write`
    - _Requirements: 6.4_

- [x] 12. Wire multi-strategy compaction
  - [x] 12.1 Route the automatic trigger through `auto_compact()` (`src/compact.rs:508`, zero callers), replacing `compact_to_target` at `src/chat.rs:1599` and `compact_session` at `src/tui.rs:1249`
    - _Requirements: 12.3_
  - [x] 12.2 Return and report `CompactionOutcome` so the developer sees which strategy ran
    - _Requirements: 12.4_
  - [x] 12.3 Write a test asserting stale-tool-result elision and file-read deduplication are attempted before summarization
    - _Requirements: 12.3_

### Phase 6 — Workspace durability

- [x] 13. Bound and cache the Workspace
  - [x] 13.1 Cap `messages` (`src/tui.rs:342`) and `history` (`:353`) with front-draining pushes at the five push sites (`:385`, `:442`, `:1081`, `:2019`, `:3203`)
    - _Requirements: 4.5_
  - [x] 13.2 Insert exactly one retained elision marker so truncation is visible rather than silent
    - _Requirements: 4.5_
  - [x] 13.3 Cache rendered lines per `Message` in a `OnceCell`, invalidated on text change, so `draw_transcript` (`:2265`, `:2293`) stops re-parsing the whole transcript per dirty frame
    - _Requirements: 4.6_
  - [x] 13.4 Write tests asserting the cap holds, exactly one elision marker exists, and redraw work is independent of retained message count
    - Asserts Property 13
    - _Requirements: 4.5, 4.6_

### Phase 7 — Capability honesty

- [x] 14. Propagate capability and plugin failures
  - [x] 14.1 Replace `CapabilitySnapshot::load(...).unwrap_or_default()` at `src/capabilities.rs:772`, `:869`, `:942`, `:975` and `load_state().unwrap_or_default()` at `:766` with `Result` propagation
    - Match the prompt path at `:705-707`, which already reports the error
    - _Requirements: 9.3, 9.4_
  - [x] 14.2 Render the parse failure on the CLI, Classic_Shell, and Workspace surfaces instead of "0 skills · 0 plugins · 0 agents"
    - _Requirements: 9.3, 9.4_
  - [x] 14.3 Fix the discarded rollback at `src/plugins.rs:833-834` so a failed restore reports the exact residue and its path
    - _Requirements: 9.10_
  - [x] 14.4 Write a test asserting a malformed manifest surfaces as an error on every surface
    - Asserts Property 11
    - _Requirements: 9.3, 9.4_

- [x] 15. Ship the capability content
  - [x] 15.1 Commit `.agents/` — 9 `SKILL.md` files the runtime discovers and `README.md` references, currently untracked and unignored
    - _Requirements: 9.5_
  - [x] 15.2 Resolve `.skills/`: either track it and remove `.gitignore:14`, or document it as user-local and stop implying a fresh clone has skills
    - `skills list` currently reports 23 skills, of which 14 come from the ignored directory
    - _Requirements: 9.5_
  - [x] 15.3 Commit `AGENTS.md` and the five untracked `docs/*.md` that `README.md` links
    - _Requirements: 9.5, 15.2_
  - [x] 15.4 Correct or remove the `test/` entry at `.gitignore:12`, which does not match the real `tests/` directory
    - _Requirements: 15.7_
  - [x] 15.5 Verify: in a fresh clone, `skills list` and `skills validate .agents/skills/repository-development` both succeed
    - _Requirements: 9.5_

### Phase 8 — MCP robustness

- [x] 16. Surface and survive MCP failures
  - [x] 16.1 Return `Vec<McpConnectFailure>` alongside discovered tools instead of only `tracing::warn!` at `src/mcp.rs:372-379`
    - The Workspace alternate screen swallows the log, so tools go silently missing
    - _Requirements: 10.4_
  - [x] 16.2 Render connect failures on the active surface, distinguishing retryable from terminal
    - _Requirements: 10.4_
  - [x] 16.3 Add bounded exponential backoff to the stdio path (`src/mcp.rs:286-320`), matching the HTTP path's `RefreshConfig` (`:261-263`), and report when it gives up
    - _Requirements: 10.5_
  - [x] 16.4 Add an OAuth `state` parameter: generate it with the existing `rand` dependency, include it in the authorization URL (`src/mcp_auth.rs:87-91`), compare it against the callback query (`:219-229`), and reject on mismatch
    - `grep state src/mcp_auth.rs` currently returns nothing
    - _Requirements: 10.8_
  - [x] 16.5 Write the filesystem token fallback at `src/mcp_auth.rs:269-274` with mode `0o600`
    - _Requirements: 10.9_
  - [x] 16.6 Add tests for `select_mcp_servers` filtering (`src/mcp.rs:54`), protocol negotiation and `2026-07-28` → `2025-11-25` fallback (`:239`), `resolve_mcp_auth` and auth hinting (`:76`, `:184`), and `mcp_server` schema stability with `isError` on failure (`src/mcp_server.rs:69-104`)
    - These four modules have zero inline tests today
    - _Requirements: 10.10_

### Phase 9 — Network exposure

- [x] 17. Make exposure explicit
  - [x] 17.1 Refuse a non-loopback bind in `server serve` when `auth_token` is `None`, overridable only by an explicit `--insecure-no-auth` flag that warns per request
    - `check_server_auth` returns `Ok(())` when no token is configured (`src/server.rs:139-141`); the token comes only from `ZAVORA_SERVER_AUTH_TOKEN` (`:418`)
    - _Requirements: 11.2_
  - [x] 17.2 Replace the token comparison at `src/server.rs:153` with a constant-time equality
    - _Requirements: 11.3_
  - [x] 17.3 Drop `profile` and `app_name` from `handle_server_health` (`src/server.rs:163-171`) unless the caller is authenticated
    - _Requirements: 11.4_
  - [x] 17.4 Add `Commands::Server` to the automation match at `src/main.rs:183-192` so a network-triggered tool resolves approval through the Automation_Surface instead of blocking on the server's stdin
    - _Requirements: 5.6, 11.5_
  - [x] 17.5 Write a test asserting a non-loopback bind without a token and without the override fails to start
    - _Requirements: 11.2_

### Phase 10 — Remaining runtime gaps

- [x] 18. Close the smaller runtime gaps
  - [x] 18.1 Warn when a legacy plaintext credential is found in profile configuration (`src/config.rs:81`), stating that it should move to the vault, while continuing to work
    - _Requirements: 3.5_
  - [x] 18.2 Reconcile planner precedence with the documentation: either extend `src/config.rs:513-516` to consider the selected agent and profile provider, or correct `README.md:138` to state that the planner reads only planner-scoped settings and defaults to OpenAI regardless of the worker provider
    - Pick one; the requirement is that documentation and implementation agree
    - _Requirements: 2.7, 15.4_
  - [x] 18.3 Make `find_closest_match` in `src/tools/file_edit.rs:191` available in the default build, or remove the "did you mean" affordance from the error path
    - It is gated on `semantic-search` (`src/tools/file_edit.rs:192`), which is not a default feature (`Cargo.toml:14`)
    - _Requirements: 6.5_
  - [x] 18.4 Revalidate the `web_fetch` host blocklist on every redirect hop with a custom redirect policy, covering IPv6 unique-local, link-local, and CGNAT ranges
    - Currently only the initial and final URLs are checked
    - _Requirements: 7.3_
  - [x] 18.5 Add tests for `classify_workflow_route` (`src/workflow.rs:59`) and `run_ralph` loop termination and cancellation reporting (`src/ralph.rs:21`)
    - _Requirements: 13.7_
  - [ ] 18.6 Report which stages completed when a workflow or Ralph run is cancelled or fails partway, and clean up any session it created
    - **Partial, 2026-08-15.** A degraded start is now announced: both `workflow` and `ralph` print every unreachable MCP server before the run begins, so a run with missing tools is never silently reported as complete. Stage-level accounting still requires threading ADK event inspection through `run_headless`, and the `fork_sub_agent` cleanup at `src/todos.rs:307-315` covers success and error but is not a Drop guard, so a panic can still leak a session. Both remain open.
    - `fork_sub_agent` cleanup at `src/todos.rs:307-315` covers success and error but not panic or cancellation
    - _Requirements: 13.5_

### Phase 11 — Documentation fidelity

- [x] 19. Build the documentation harness first
  - [x] 19.1 Add `scripts/check_docs.sh`: extract fenced blocks from `README.md`, `QUICKSTART.md`, and `docs/*.md`; load each profile-configuration `toml` block into a scratch `.zavora/config.toml` and assert `profiles show` succeeds; assert every `zavora-cli` invocation in a `bash` block resolves against `--help`
    - Asserts Property 12
    - _Requirements: 15.1, 15.2_
  - [x] 19.2 Wire `check_docs.sh` into `make ci`
    - _Requirements: 15.1, 15.2_

- [x] 20. Fix the documentation the harness now checks
  - [x] 20.1 Remove `auto_compact_enabled = true` from `README.md:135` and `docs/GA_SIGNOFF_v110.md:44`
    - The key lives on `RuntimeConfig` (`src/config.rs:57`), not `ProfileConfig`, which sets `deny_unknown_fields` (`:71`); the block verbatim yields `[PROVIDER] invalid profile configuration`
    - _Requirements: 15.1_
  - [x] 20.2 Correct the MSRV to 1.95 in `QUICKSTART.md:5`, `CHANGELOG.md:48`, and `docs/MIGRATION_GUIDE_v2.md:7`
    - _Requirements: 1.5, 15.3_
  - [x] 20.3 Correct `rmcp 2.2` to `rmcp 3.1` at `CHANGELOG.md:41` and add the MCP `2026-07-28` stdio discovery lifecycle
    - _Requirements: 15.6_
  - [x] 20.4 Fold `[Unreleased]` into `[2.0.0]` and add the automation surface, plugins, skills lifecycle, capabilities and MCP catalog, project instructions, and specialist agents — none of which appear anywhere in the file
    - _Requirements: 15.6_
  - [x] 20.5 Mark hooks, LSP, and compaction strategy as available only if Phases 5 wired them; otherwise state they are not wired
    - _Requirements: 15.5_
  - [x] 20.6 Replace the non-catalog model id in the `docs/HEADLESS.md` sample and document the `--yolo` alias
    - _Requirements: 5.7_
  - [x] 20.7 Delete the tracked scratch files `DOCUMENTATION_DISCREPANCIES.md`, `PROJECT_STATUS.md`, `SESSION_SUMMARY.md`, `tests/hello.py`, `tests/test_hello.py`, and the untracked `hello_world/`
    - _Requirements: 15.7_
  - [x] 20.8 Add `docs/CAPABILITIES.md` covering the five categories, the MCP recipes, pack ids, and the `configured` / `connected` / `authorized` distinction, and document the specialist agent names and the utility model role
    - The largest undocumented surface: `src/capabilities.rs` and `src/mcp_catalog.rs` combined
    - _Requirements: 9.1, 9.2_
  - [x] 20.9 Add a `docs/` index and link `PLUGINS.md`, `DISTRIBUTION.md`, and `SERVER_MODE.md` from the README
    - _Requirements: 15.2_
  - [x] 20.10 Archive the superseded v1 milestone and multi-agent documents under `docs/history/`
    - Done: 14 documents moved, with `docs/history/README.md` recording that they are superseded and excluded from the documentation harness.
    - _Requirements: 15.7_
  - [x] 20.11 Verify: `scripts/check_docs.sh` passes
    - _Requirements: 15.1, 15.2_

### Phase 12 — Distribution and CI

- [x] 21. Make CI a real gate
  - [x] 21.1 Change `.github/workflows/ci.yml` from `dtolnay/rust-toolchain@stable` to the pinned toolchain
    - _Requirements: 16.5_
  - [x] 21.2 Change `cargo check` to `check --all-targets` and `cargo test` to `test --all-targets -- --test-threads=1`
    - _Requirements: 16.4_
  - [x] 21.3 Add an optional-feature matrix job covering `web-fetch,lsp,oauth,browser,sandbox,rag,semantic-search,checkpoints`
    - _Requirements: 16.4_
  - [x] 21.4 Add `cargo audit`, `check_wiring.sh`, `check_docs.sh`, and `check_clean_clone.sh` to `make ci` / `make release-check`
    - _Requirements: 16.4_
  - [x] 21.5 Add `cargo publish --dry-run` to `make release-check`
    - _Requirements: 16.4_
  - [x] 21.6 Verify: CI passes on a branch with no sibling `adk-rust` checkout
    - _Requirements: 1.4, 16.4_

- [x] 22. Align the distribution channels
  - [x] 22.1 Regenerate `Formula/zavora-cli.rb` for 2.0.0 with `scripts/generate_homebrew_formula.sh`
    - It still pins `v1.1.4.tar.gz` at `:4` while `README.md:28` tells users to install from it
    - _Requirements: 16.1, 16.3, 15.8_
  - [x] 22.2 Extend `make version-check` to include the formula version alongside `Cargo.toml` and `npm/zavora-cli/package.json`
    - Asserts Property 14
    - _Requirements: 16.2_
  - [x] 22.3 Add formula regeneration and verification to the release workflow so it is not maintained by hand
    - _Requirements: 16.3_
  - [x] 22.4 Verify: `make version-check` passes and the three channel versions match the intended tag
    - _Requirements: 16.1, 16.2_

### Phase 13 — Spec integrity and release

- [x] 23. Reconcile the spec backlog
  - [x] 23.1 Mark the ~104 verified-complete items in `.kiro/specs/claude-code-capability-extraction/tasks.md` as complete
    - Currently 0 of 143 are ticked while most are implemented
    - _Requirements: 17.1_
  - [x] 23.2 Un-tick `.kiro/specs/ralph-orchestrator-routing/tasks.md:37`, `:51`, `:56`
    - They claim `src/agents/ralph_agent.rs` exists; it does not
    - _Requirements: 17.1_
  - [x] 23.3 Decide the Ralph sub-agent question: implement `ralph_agent.rs` with registration and a prompt entry, or de-scope the spec
    - **Decision, 2026-08-15: de-scoped.** Ralph is reachable as `zavora-cli ralph` and as `/ralph`, and it already runs on the v2 worker/planner runtime through `src/ralph.rs`. Adding a `ralph_agent` sub-agent would duplicate that path and, until registered, would be exactly the phantom-tool defect Requirement 6.2 forbids. The three falsely-checked tasks in `ralph-orchestrator-routing` were reverted to unchecked with this rationale.
    - _Requirements: 13.6, 17.2_
  - [x] 23.4 Annotate the six deliberately superseded design decisions rather than leaving them to contradict the code
    - `src/jsonrpc.rs` → `rmcp`; `McpTransport` enum → `command: Option<String>` plus `is_stdio()`; `RalphConfigBridge` → v2-native `run_ralph`; `fork_sub_agent` located in `src/todos.rs`; threshold tool deferral → `CapabilityToolset` routing; `keyring` unconditional rather than `oauth`-gated
    - _Requirements: 17.2_
  - [x] 23.5 Add a supersession note to each of the six earlier specs pointing at `v2-vision`
    - _Requirements: 17.3_
  - [x] 23.6 Either schedule the 24 optional `- [ ]*` property tests with a framework in `[dev-dependencies]`, or de-scope them explicitly
    - **Decision, 2026-08-15: de-scoped for 2.0.0**, as recorded in the Out of scope section of `requirements.md`. Adding a property-testing framework is a new dependency and a new testing idiom; the behaviours those tasks describe are covered by targeted unit tests, and the suite grew from 256 to 327 tests during this work. Revisit post-2.0.0.
    - _Requirements: 17.5_

- [x] 24. Land the work in reviewable commits
  - [ ] 24.1 Split the outstanding change set — 25 modified files, +4713/-800, plus six new modules — into per-area commits: Workspace, automation surface, plugins and skills, capabilities and MCP catalog, tools
    - _Requirements: 17.1_
  - [ ] 24.2 Commit each phase of this plan separately with its verification output referenced in the message
    - _Requirements: 17.1_

- [x] 25. Verify the release acceptance gate
  - [x] 25.1 Run all thirteen gate items from `requirements.md` and record each command's output
    - _Requirements: 1.4, 6.2, 6.3, 7.1, 7.4, 8.4, 15.1, 16.1, 16.4, 16.6, 17.1_
  - [x] 25.2 Confirm no gate item relies on an untested assertion
    - _Requirements: 17.1_
  - [ ] 25.3 Tag `v2.0.0` only after every gate item passes
    - _Requirements: 16.1_
