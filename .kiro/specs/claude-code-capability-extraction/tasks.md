# Tasks: Claude Code Capability Extraction for Zavora-CLI

> **Superseded by [`.kiro/specs/v2-vision`](../v2-vision/requirements.md) on 2026-08-15.**
> That spec is the authoritative v2 contract and holds the reconciled backlog.
> This document is retained for provenance. Checkbox state here was reconciled
> against the runtime on 2026-08-15; where the implementation deliberately
> diverged, the divergence is recorded in v2-vision task 23.4.

## Reconciliation, 2026-08-15

Checkbox state in this document was wrong in one direction: the work was largely
implemented and never ticked. It has been reconciled against the runtime.

Six design decisions were deliberately superseded and are **not** open work:

| Spec proposal | Implemented instead |
|---|---|
| shared `src/jsonrpc.rs` | `rmcp` 3.1 for MCP; inline framing in `src/lsp/client.rs` |
| `McpTransport` enum | `command: Option<String>` plus `McpServerConfig::is_stdio()` |
| `RalphConfigBridge` | v2-native `run_ralph` on the worker/planner runtime |
| `fork_sub_agent` in `runner.rs` | implemented in `src/todos.rs` |
| threshold-based tool deferral | per-prompt `CapabilityToolset` routing |
| `keyring` gated on `oauth` | unconditional dependency |

Items that remain unchecked are genuinely open, or are optional `- [ ]*` test
tasks blocked on a property-testing framework that is out of scope for 2.0.0.
See [`.kiro/specs/v2-vision`](../v2-vision/tasks.md) for the authoritative backlog.

## Phase 1 — Foundation Tools

### Task 1: String-Replace File Edit Tool (REQ-3)
- [x] Create `src/tools/file_edit.rs` with `FileEditRequest` struct (file_path, old_string, new_string, replace_all)
- [x] Add `pub mod file_edit;` to `src/tools/mod.rs`
- [x] Implement path resolution reusing `fs_read::enforce_workspace_path_policy()`
- [x] Reject files larger than 10MB; return error if old_string == new_string
- [x] Implement occurrence counting — error if 0 matches (with closest-match hint via `strsim`), error if >1 match and `replace_all=false`
- [x] Implement replacement with line-ending preservation (detect LF vs CRLF before edit)
- [x] Generate unified diff output using `similar::TextDiff`
- [x] Register `"file_edit"` in `build_builtin_tools()` in `src/tools/mod.rs`
- [x] Add `file_edit` tool description to the system prompt in `src/runner.rs` as the preferred edit tool
- [x] Verify existing `fs_write` patch mode still works (backward compat)

### Task 2: Glob Tool (REQ-4)
- [x] Add `ignore = "0.4"` to `Cargo.toml` dependencies
- [x] Create `src/tools/glob.rs` with `GlobRequest` (pattern, optional path) and `GlobOutput` (num_files, filenames, truncated, duration_ms)
- [x] Add `pub mod glob;` to `src/tools/mod.rs`
- [x] Implement directory walk using `ignore::WalkBuilder` (respects `.gitignore`) with built-in globset matching
- [x] Match entries against glob pattern, collect up to 100 results, set `truncated` flag
- [x] Return paths relative to cwd
- [x] Enforce workspace path policy on search root
- [x] Register `"glob"` in `build_builtin_tools()`
- [x] Add glob tool description to system prompt in `src/runner.rs`

### Task 3: Grep Tool (REQ-5)
- [x] Create `src/tools/grep.rs` with `GrepRequest` struct (pattern, path, glob, file_type, output_mode, context lines, case_insensitive, head_limit, offset, multiline) and `GrepOutput` struct
- [x] Add `pub mod grep;` to `src/tools/mod.rs`
- [x] Implement `rg` invocation: build args from request, always exclude `.git`
- [x] Parse `rg` stdout, apply offset + head_limit truncation
- [x] Return structured output with match/file counts and truncation status
- [x] Implement `grep -rn` fallback when `rg` is not in PATH
- [x] Enforce workspace path policy
- [x] Register `"grep"` in `build_builtin_tools()`
- [x] Add grep tool description to system prompt in `src/runner.rs`

### Task 4: Web Fetch Tool (REQ-8)
- [x] Add `reqwest` and `htmd` (or `html2text`) as optional dependencies behind `web-fetch` feature flag
- [x] Create `src/tools/web_fetch.rs` with `WebFetchRequest` (url, prompt) and `WebFetchOutput`
- [x] Add `pub mod web_fetch;` to `src/tools/mod.rs` (gated on feature)
- [x] Implement domain blocklist (localhost, metadata endpoints, private IPs) — check each redirect hop
- [x] Implement HTTP GET with 30s timeout, max 5 redirects, `User-Agent: zavora-cli/<version>`
- [x] Handle content types: HTML→markdown, JSON→pretty-print, text→raw
- [x] Truncate to 100KB
- [x] Return structured output with status code, bytes, duration; include prompt as context for LLM
- [x] Register `"web_fetch"` in `build_builtin_tools()` (gated on feature flag)
- [x] Mark as requires-confirmation (not auto-approved)
- [x] Add web_fetch tool description to system prompt in `src/runner.rs`

---

## Phase 2 — MCP Infrastructure

### Task 5: MCP Server Mode (REQ-1)
- [ ] Create `src/jsonrpc.rs` — shared Content-Length framed JSON-RPC read/write (reused by MCP server, MCP stdio client, LSP client)
  - **Obsolete.** Superseded by `rmcp` 3.1 and inline framing in `src/lsp/client.rs`.
- [x] Evaluate Rust MCP SDK options (`rmcp` crate or minimal hand-rolled JSON-RPC)
- [x] Create `src/mcp_server.rs` implementing `initialize`, `tools/list`, and `tools/call` handlers over stdio
- [x] Implement `initialize` handler returning server capabilities `{ tools: {} }` and server info
- [x] Map each ADK `Tool` to MCP tool definition (name, description, JSON Schema from `parameters_schema()`)
- [x] Implement tool call dispatch: find tool by name, deserialize args, call, serialize result
- [x] Implement error handling: return `{ isError: true, content: [{ type: "text", text: error_message }] }` on tool failure
- [x] Bypass interactive permission checks; preserve tool-level input validation (workspace path policy)
- [x] Add `McpCommands::Serve` variant to `src/cli.rs`
- [x] Wire `zavora mcp serve` to `run_mcp_server()` in `src/main.rs`
- [ ] Add `pub mod jsonrpc;` and `pub mod mcp_server;` to `src/lib.rs`
  - **Partly obsolete.** `mcp_server` is registered; `jsonrpc` will not exist.
- [ ] Test with an MCP client (e.g., Claude Desktop config pointing to `zavora mcp serve`)
  - **Open — manual.** Needs a real external client; not automatable in CI.

### Task 6: MCP Stdio Client Transport (REQ-2)
- [x] Refactor `McpServerConfig` in `src/config.rs` to use `McpTransport` enum (Http/Stdio) with backward-compatible deserialization
- [x] Implement `StdioMcpClient` in `src/mcp.rs`: spawn child process, use shared `jsonrpc.rs` for Content-Length framed communication
- [x] Implement process lifecycle: spawn, initialize, reconnect on crash (exponential backoff 500ms→30s, max 5 attempts)
- [x] Implement `discover_mcp_tools_for_stdio_server()` parallel to existing HTTP discovery
- [x] Update `discover_mcp_tools()` to dispatch based on transport type
- [ ] Update config documentation and `.env.example`
  - **Open.** README documents stdio; `.env.example` has no MCP section.
- [ ] Test with a stdio MCP server (e.g., `npx @modelcontextprotocol/server-filesystem`)
  - **Open — manual.** Needs a real external server.

### Task 7: Layered Permission System (REQ-9)
- [x] Define `PermissionRules` struct with `always_allow`, `always_deny`, `always_ask` pattern lists in `src/tool_policy.rs`
- [x] Implement glob pattern matching for tool name + content patterns
- [x] Add `is_read_only()` and `is_concurrency_safe()` methods to tool wrappers
- [x] Refactor `src/tools/confirming.rs` into pipeline: validate → hooks → rules → tool-specific check → default
- [x] Auto-approve read-only tools by default
- [x] Add `permission_rules` section to `ProfileConfig` in `src/config.rs`
- [x] Map existing `approve_tool`/`require_confirm_tool` into new rule format
- [x] Add `/allow <pattern>` and `/deny <pattern>` slash commands in `src/chat.rs` for session-level rule overrides
- [x] Verify backward compatibility with existing confirmation behavior

---

## Phase 3 — Security & Intelligence

### Task 8: Bash Security Validation Layer (REQ-6)
- [x] Create `src/tools/bash_security.rs` with `SecurityResult` enum and `ValidationContext` struct (including all 4 quote-extraction variants)
- [x] Implement `build_validation_context()`: parse base command, run quote extraction producing `unquoted_content`, `fully_unquoted`, `fully_unquoted_pre_strip`, `unquoted_keep_quote_chars`
- [x] Implement `strip_safe_redirections()` helper with trailing boundary assertions to prevent prefix matching
- [x] Implement `validate_empty()` — allow empty commands
- [x] Implement `validate_incomplete_commands()` — deny commands starting with tab, flags, or continuation operators
- [x] Implement `validate_command_substitution()` — deny `$()`, backticks, `<()`, `>()`, `${}`, `$[]` in unquoted content; allow safe heredoc patterns
- [x] Implement `validate_shell_metacharacters()` — deny unquoted `|`, `&`, `;`; allow `2>&1`, `> /dev/null`
- [x] Implement `validate_dangerous_variables()` — deny `IFS=`, `PATH=`, `LD_PRELOAD=`, `LD_LIBRARY_PATH=`
- [x] Implement `validate_newlines()` — deny literal `\n` in commands
- [x] Implement `validate_redirections()` — deny output redirections except safe patterns
- [ ] Implement `validate_heredoc_safety()` — allow only single-quoted/escaped delimiter heredocs in `$(cat <<'DELIM')` form
  - **Open.** The only unimplemented validator; `$(` is denied unconditionally today, which is safe but stricter than specified.
- [x] Implement `validate_obfuscated_flags()` — deny flags containing shell metacharacters or non-ASCII
- [x] Implement `validate_brace_expansion()` — deny `{a,b}` and `{1..10}` in `fully_unquoted_pre_strip`
- [x] Implement `validate_unicode_whitespace()` — deny non-ASCII whitespace (U+00A0, U+2000–U+200F, etc.)
- [x] Implement `validate_carriage_return()` — deny `\r` characters
- [x] Implement `validate_proc_environ_access()` — deny `/proc/*/environ`, `/proc/*/cmdline`
- [x] Implement `validate_ifs_injection()` — deny IFS manipulation
- [x] Implement `validate_backslash_escaped_operators()` — deny `\|`, `\&`, `\;`
- [x] Implement `validate_comment_quote_desync()` — deny `#` adjacent to closing quotes in `unquoted_keep_quote_chars`
- [x] Implement `validate_mid_word_hash()` — deny `#` mid-word outside quotes in `unquoted_keep_quote_chars`
- [x] Implement `validate_malformed_token_injection()` — deny tokens exploiting parser bugs through malformed quoting
- [x] Implement `validate_jq_system_function()` — deny jq `system()`, `@sh` functions
- [x] Implement `validate_git_commit_substitution()` — deny `$()` / backticks in git commit messages
- [x] Wire `validate_bash_command()` pipeline into `execute_bash` tool, replacing flat `DENIED_PATTERNS`
- [x] Preserve `READONLY_COMMANDS` as early-exit auto-approve before security pipeline
- [x] Add unit tests for each validation function with attack vectors

### Task 9: LSP Tool (REQ-7)
- [x] Create `src/lsp/mod.rs`, `src/lsp/manager.rs`, `src/lsp/client.rs`
- [x] Implement `LspClient`: JSON-RPC over stdio using shared `jsonrpc.rs` Content-Length framing, request/response correlation via ID
- [x] Implement `LspManager`: HashMap of language→server handle, lazy start, file tracking, crash counter (max 3 restarts per language)
- [x] Implement language detection from file extension (Rust, TypeScript, Python, Go, Java, Ruby, C/C++)
- [x] Implement LSP initialization handshake: `initialize` request with workspace root, `initialized` notification
- [x] Implement `textDocument/didOpen` for file sync before requests
- [ ] Implement `textDocument/didChange` notification hook — called by file_edit/fs_write when they modify a file that's open in an LSP server
  - **Open.** `notify_file_changed` exists but has no caller, so server state goes stale after an edit.
- [ ] Implement `textDocument/didClose` for batch cleanup on session end
  - **Open.**
- [x] Implement 9 operations: goToDefinition, findReferences, hover, documentSymbol, workspaceSymbol, goToImplementation, prepareCallHierarchy, incomingCalls, outgoingCalls
- [x] Implement two-step call hierarchy: prepareCallHierarchy → incomingCalls/outgoingCalls
- [x] Implement result formatting: relative paths, line numbers, symbol kinds
- [ ] Implement `.gitignore` filtering on location results
  - **Open.** Noise reduction only.
- [ ] Implement crash recovery: detect server exit, mark as dead, auto-restart on next call (max 3 per session)
  - **Partial.** stderr is now captured; exit detection and bounded restart are not implemented.
- [x] Pipe LSP server stderr to `tracing::debug` for diagnostics
- [x] Create `src/tools/lsp.rs` tool wrapper with input validation (file exists, <10MB, valid operation)
- [x] Add LSP config loading from `.zavora/lsp.json`
- [x] Add `zavora lsp init` command to generate default config (only for servers found in PATH)
- [ ] Register `"lsp"` in `build_builtin_tools()` (gated on config existence)
  - **Divergence.** Gated on the `lsp` feature rather than config existence; `init_manager()` now runs during surface assembly.
- [x] Implement graceful shutdown: `shutdown` request + `exit` notification on session end
- [x] Add `pub mod lsp;` to `src/lib.rs`
- [x] Update `src/doctor.rs` to check for language server binaries in PATH

---

## Phase 4 — Advanced Patterns

### Task 10: Parallel Tool Execution (REQ-10)
- [x] Add `is_concurrency_safe()` and `is_read_only()` to tool trait/wrapper
- [x] Mark safe tools: fs_read, glob, grep, lsp, current_unix_time, todo_list (read ops)
- [ ] Implement contiguous-group partitioning: identify runs of consecutive read-only tools, keep serial tools in original position
  - **Out of scope.** Lives upstream in `adk-agent`.
- [ ] Execute each parallel group with `futures::future::join_all` (max 10 concurrent), serial groups sequentially
  - **Out of scope.** Upstream in `adk-agent`.
- [x] Collect all results independently (parallel errors don't abort siblings)
- [ ] Assemble results in original LLM-requested order regardless of execution order
  - **Out of scope.** Upstream in `adk-agent`.
- [ ] Verify correct result ordering when returning to the LLM
  - **Out of scope.** Upstream in `adk-agent`.

### Task 11: Fork Sub-Agents (REQ-11)
- [x] Implement `fork_sub_agent()` in `src/runner.rs`: create temp session (using timestamp-based ID, no uuid), build agent with tools, run prompt with optional file context, enforce timeout (default 5 min)
- [ ] Wire into orchestrator agent for task delegation
  - **Deferred.** `/delegate` is user-invoked; an LLM-callable delegate tool is post-2.0.0.
- [ ] Sub-agent uses parent's model by default; allow override via agent config
  - **Partial.** The model is inherited; there is no per-delegate override field.
- [x] Ensure sub-agent inherits permission rules and security validation
- [ ] Add session cleanup on completion and on error (Drop guard pattern)
  - **Partial.** Cleanup covers success and error paths but is not a Drop guard, so a panic can leak the session.

### Task 12: Multi-Strategy Compaction (REQ-12)
- [x] Implement `snip_stale_tool_results()` in `src/compact.rs`: identify large/old/failed tool results for removal, plus dedup of file read results (keep most recent per path)
- [x] Implement `auto_compact()`: try snip first, fall back to summary if still over threshold
- [ ] Add `compaction_strategy` config option (summary/snip/auto)
  - **Deferred.** Multi-strategy compaction runs without a switch.
- [x] Wire auto strategy into the existing auto-compaction trigger

### Task 13: MCP OAuth (REQ-13)
- [ ] Add `keyring` as optional dependency behind `oauth` feature flag
  - **Divergence.** `keyring` is an unconditional dependency; credential storage is not optional.
- [x] Create `src/mcp_auth.rs` with OAuth 2.0 + PKCE flow
- [x] Implement auth server metadata discovery (`.well-known/oauth-authorization-server`)
- [x] Implement browser-based authorization with localhost callback listener
- [x] Implement token exchange, storage (keyring), and automatic refresh
- [x] Add `oauth` field to `McpServerConfig`
- [x] Wire OAuth into MCP client connection flow (both HTTP and stdio)

### Task 14: Tool Search (REQ-14)
- [x] Create `src/tools/tool_search.rs` with case-insensitive keyword-based tool discovery
- [x] Implement search across tool names and descriptions
- [ ] Add `tool_search_enabled` config option
  - **Deferred.** The tool registers above a fixed threshold.
- [ ] When enabled and tool count > 20, include only core tools (fs_read, fs_write, file_edit, execute_bash, glob, grep, tool_search) in system prompt, defer others
  - **Divergence.** Replaced by per-prompt `CapabilityToolset` routing.
- [ ] Implement tool re-registration: after tool_search returns results, promote matching deferred tools to active set so LLM can call them in subsequent turns
  - **Divergence.** Superseded by `CapabilityToolset` routing.
- [x] Register `"tool_search"` in `build_builtin_tools()`
