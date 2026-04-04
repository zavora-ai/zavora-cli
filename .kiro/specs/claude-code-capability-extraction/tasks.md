# Tasks: Claude Code Capability Extraction for Zavora-CLI

## Phase 1 — Foundation Tools

### Task 1: String-Replace File Edit Tool (REQ-3)
- [ ] Create `src/tools/file_edit.rs` with `FileEditRequest` struct (file_path, old_string, new_string, replace_all)
- [ ] Add `pub mod file_edit;` to `src/tools/mod.rs`
- [ ] Implement path resolution reusing `fs_read::enforce_workspace_path_policy()`
- [ ] Reject files larger than 10MB; return error if old_string == new_string
- [ ] Implement occurrence counting — error if 0 matches (with closest-match hint via `strsim`), error if >1 match and `replace_all=false`
- [ ] Implement replacement with line-ending preservation (detect LF vs CRLF before edit)
- [ ] Generate unified diff output using `similar::TextDiff`
- [ ] Register `"file_edit"` in `build_builtin_tools()` in `src/tools/mod.rs`
- [ ] Add `file_edit` tool description to the system prompt in `src/runner.rs` as the preferred edit tool
- [ ] Verify existing `fs_write` patch mode still works (backward compat)

### Task 2: Glob Tool (REQ-4)
- [ ] Add `ignore = "0.4"` to `Cargo.toml` dependencies
- [ ] Create `src/tools/glob.rs` with `GlobRequest` (pattern, optional path) and `GlobOutput` (num_files, filenames, truncated, duration_ms)
- [ ] Add `pub mod glob;` to `src/tools/mod.rs`
- [ ] Implement directory walk using `ignore::WalkBuilder` (respects `.gitignore`) with built-in globset matching
- [ ] Match entries against glob pattern, collect up to 100 results, set `truncated` flag
- [ ] Return paths relative to cwd
- [ ] Enforce workspace path policy on search root
- [ ] Register `"glob"` in `build_builtin_tools()`
- [ ] Add glob tool description to system prompt in `src/runner.rs`

### Task 3: Grep Tool (REQ-5)
- [ ] Create `src/tools/grep.rs` with `GrepRequest` struct (pattern, path, glob, file_type, output_mode, context lines, case_insensitive, head_limit, offset, multiline) and `GrepOutput` struct
- [ ] Add `pub mod grep;` to `src/tools/mod.rs`
- [ ] Implement `rg` invocation: build args from request, always exclude `.git`
- [ ] Parse `rg` stdout, apply offset + head_limit truncation
- [ ] Return structured output with match/file counts and truncation status
- [ ] Implement `grep -rn` fallback when `rg` is not in PATH
- [ ] Enforce workspace path policy
- [ ] Register `"grep"` in `build_builtin_tools()`
- [ ] Add grep tool description to system prompt in `src/runner.rs`

### Task 4: Web Fetch Tool (REQ-8)
- [ ] Add `reqwest` and `htmd` (or `html2text`) as optional dependencies behind `web-fetch` feature flag
- [ ] Create `src/tools/web_fetch.rs` with `WebFetchRequest` (url, prompt) and `WebFetchOutput`
- [ ] Add `pub mod web_fetch;` to `src/tools/mod.rs` (gated on feature)
- [ ] Implement domain blocklist (localhost, metadata endpoints, private IPs) — check each redirect hop
- [ ] Implement HTTP GET with 30s timeout, max 5 redirects, `User-Agent: zavora-cli/<version>`
- [ ] Handle content types: HTML→markdown, JSON→pretty-print, text→raw
- [ ] Truncate to 100KB
- [ ] Return structured output with status code, bytes, duration; include prompt as context for LLM
- [ ] Register `"web_fetch"` in `build_builtin_tools()` (gated on feature flag)
- [ ] Mark as requires-confirmation (not auto-approved)
- [ ] Add web_fetch tool description to system prompt in `src/runner.rs`

---

## Phase 2 — MCP Infrastructure

### Task 5: MCP Server Mode (REQ-1)
- [ ] Create `src/jsonrpc.rs` — shared Content-Length framed JSON-RPC read/write (reused by MCP server, MCP stdio client, LSP client)
- [ ] Evaluate Rust MCP SDK options (`rmcp` crate or minimal hand-rolled JSON-RPC)
- [ ] Create `src/mcp_server.rs` implementing `initialize`, `tools/list`, and `tools/call` handlers over stdio
- [ ] Implement `initialize` handler returning server capabilities `{ tools: {} }` and server info
- [ ] Map each ADK `Tool` to MCP tool definition (name, description, JSON Schema from `parameters_schema()`)
- [ ] Implement tool call dispatch: find tool by name, deserialize args, call, serialize result
- [ ] Implement error handling: return `{ isError: true, content: [{ type: "text", text: error_message }] }` on tool failure
- [ ] Bypass interactive permission checks; preserve tool-level input validation (workspace path policy)
- [ ] Add `McpCommands::Serve` variant to `src/cli.rs`
- [ ] Wire `zavora mcp serve` to `run_mcp_server()` in `src/main.rs`
- [ ] Add `pub mod jsonrpc;` and `pub mod mcp_server;` to `src/lib.rs`
- [ ] Test with an MCP client (e.g., Claude Desktop config pointing to `zavora mcp serve`)

### Task 6: MCP Stdio Client Transport (REQ-2)
- [ ] Refactor `McpServerConfig` in `src/config.rs` to use `McpTransport` enum (Http/Stdio) with backward-compatible deserialization
- [ ] Implement `StdioMcpClient` in `src/mcp.rs`: spawn child process, use shared `jsonrpc.rs` for Content-Length framed communication
- [ ] Implement process lifecycle: spawn, initialize, reconnect on crash (exponential backoff 500ms→30s, max 5 attempts)
- [ ] Implement `discover_mcp_tools_for_stdio_server()` parallel to existing HTTP discovery
- [ ] Update `discover_mcp_tools()` to dispatch based on transport type
- [ ] Update config documentation and `.env.example`
- [ ] Test with a stdio MCP server (e.g., `npx @modelcontextprotocol/server-filesystem`)

### Task 7: Layered Permission System (REQ-9)
- [ ] Define `PermissionRules` struct with `always_allow`, `always_deny`, `always_ask` pattern lists in `src/tool_policy.rs`
- [ ] Implement glob pattern matching for tool name + content patterns
- [ ] Add `is_read_only()` and `is_concurrency_safe()` methods to tool wrappers
- [ ] Refactor `src/tools/confirming.rs` into pipeline: validate → hooks → rules → tool-specific check → default
- [ ] Auto-approve read-only tools by default
- [ ] Add `permission_rules` section to `ProfileConfig` in `src/config.rs`
- [ ] Map existing `approve_tool`/`require_confirm_tool` into new rule format
- [ ] Add `/allow <pattern>` and `/deny <pattern>` slash commands in `src/chat.rs` for session-level rule overrides
- [ ] Verify backward compatibility with existing confirmation behavior

---

## Phase 3 — Security & Intelligence

### Task 8: Bash Security Validation Layer (REQ-6)
- [ ] Create `src/tools/bash_security.rs` with `SecurityResult` enum and `ValidationContext` struct (including all 4 quote-extraction variants)
- [ ] Implement `build_validation_context()`: parse base command, run quote extraction producing `unquoted_content`, `fully_unquoted`, `fully_unquoted_pre_strip`, `unquoted_keep_quote_chars`
- [ ] Implement `strip_safe_redirections()` helper with trailing boundary assertions to prevent prefix matching
- [ ] Implement `validate_empty()` — allow empty commands
- [ ] Implement `validate_incomplete_commands()` — deny commands starting with tab, flags, or continuation operators
- [ ] Implement `validate_command_substitution()` — deny `$()`, backticks, `<()`, `>()`, `${}`, `$[]` in unquoted content; allow safe heredoc patterns
- [ ] Implement `validate_shell_metacharacters()` — deny unquoted `|`, `&`, `;`; allow `2>&1`, `> /dev/null`
- [ ] Implement `validate_dangerous_variables()` — deny `IFS=`, `PATH=`, `LD_PRELOAD=`, `LD_LIBRARY_PATH=`
- [ ] Implement `validate_newlines()` — deny literal `\n` in commands
- [ ] Implement `validate_redirections()` — deny output redirections except safe patterns
- [ ] Implement `validate_heredoc_safety()` — allow only single-quoted/escaped delimiter heredocs in `$(cat <<'DELIM')` form
- [ ] Implement `validate_obfuscated_flags()` — deny flags containing shell metacharacters or non-ASCII
- [ ] Implement `validate_brace_expansion()` — deny `{a,b}` and `{1..10}` in `fully_unquoted_pre_strip`
- [ ] Implement `validate_unicode_whitespace()` — deny non-ASCII whitespace (U+00A0, U+2000–U+200F, etc.)
- [ ] Implement `validate_carriage_return()` — deny `\r` characters
- [ ] Implement `validate_proc_environ_access()` — deny `/proc/*/environ`, `/proc/*/cmdline`
- [ ] Implement `validate_ifs_injection()` — deny IFS manipulation
- [ ] Implement `validate_backslash_escaped_operators()` — deny `\|`, `\&`, `\;`
- [ ] Implement `validate_comment_quote_desync()` — deny `#` adjacent to closing quotes in `unquoted_keep_quote_chars`
- [ ] Implement `validate_mid_word_hash()` — deny `#` mid-word outside quotes in `unquoted_keep_quote_chars`
- [ ] Implement `validate_malformed_token_injection()` — deny tokens exploiting parser bugs through malformed quoting
- [ ] Implement `validate_jq_system_function()` — deny jq `system()`, `@sh` functions
- [ ] Implement `validate_git_commit_substitution()` — deny `$()` / backticks in git commit messages
- [ ] Wire `validate_bash_command()` pipeline into `execute_bash` tool, replacing flat `DENIED_PATTERNS`
- [ ] Preserve `READONLY_COMMANDS` as early-exit auto-approve before security pipeline
- [ ] Add unit tests for each validation function with attack vectors

### Task 9: LSP Tool (REQ-7)
- [ ] Create `src/lsp/mod.rs`, `src/lsp/manager.rs`, `src/lsp/client.rs`
- [ ] Implement `LspClient`: JSON-RPC over stdio using shared `jsonrpc.rs` Content-Length framing, request/response correlation via ID
- [ ] Implement `LspManager`: HashMap of language→server handle, lazy start, file tracking, crash counter (max 3 restarts per language)
- [ ] Implement language detection from file extension (Rust, TypeScript, Python, Go, Java, Ruby, C/C++)
- [ ] Implement LSP initialization handshake: `initialize` request with workspace root, `initialized` notification
- [ ] Implement `textDocument/didOpen` for file sync before requests
- [ ] Implement `textDocument/didChange` notification hook — called by file_edit/fs_write when they modify a file that's open in an LSP server
- [ ] Implement `textDocument/didClose` for batch cleanup on session end
- [ ] Implement 9 operations: goToDefinition, findReferences, hover, documentSymbol, workspaceSymbol, goToImplementation, prepareCallHierarchy, incomingCalls, outgoingCalls
- [ ] Implement two-step call hierarchy: prepareCallHierarchy → incomingCalls/outgoingCalls
- [ ] Implement result formatting: relative paths, line numbers, symbol kinds
- [ ] Implement `.gitignore` filtering on location results
- [ ] Implement crash recovery: detect server exit, mark as dead, auto-restart on next call (max 3 per session)
- [ ] Pipe LSP server stderr to `tracing::debug` for diagnostics
- [ ] Create `src/tools/lsp.rs` tool wrapper with input validation (file exists, <10MB, valid operation)
- [ ] Add LSP config loading from `.zavora/lsp.json`
- [ ] Add `zavora lsp init` command to generate default config (only for servers found in PATH)
- [ ] Register `"lsp"` in `build_builtin_tools()` (gated on config existence)
- [ ] Implement graceful shutdown: `shutdown` request + `exit` notification on session end
- [ ] Add `pub mod lsp;` to `src/lib.rs`
- [ ] Update `src/doctor.rs` to check for language server binaries in PATH

---

## Phase 4 — Advanced Patterns

### Task 10: Parallel Tool Execution (REQ-10)
- [ ] Add `is_concurrency_safe()` and `is_read_only()` to tool trait/wrapper
- [ ] Mark safe tools: fs_read, glob, grep, lsp, current_unix_time, todo_list (read ops)
- [ ] Implement contiguous-group partitioning: identify runs of consecutive read-only tools, keep serial tools in original position
- [ ] Execute each parallel group with `futures::future::join_all` (max 10 concurrent), serial groups sequentially
- [ ] Collect all results independently (parallel errors don't abort siblings)
- [ ] Assemble results in original LLM-requested order regardless of execution order
- [ ] Verify correct result ordering when returning to the LLM

### Task 11: Fork Sub-Agents (REQ-11)
- [ ] Implement `fork_sub_agent()` in `src/runner.rs`: create temp session (using timestamp-based ID, no uuid), build agent with tools, run prompt with optional file context, enforce timeout (default 5 min)
- [ ] Wire into orchestrator agent for task delegation
- [ ] Sub-agent uses parent's model by default; allow override via agent config
- [ ] Ensure sub-agent inherits permission rules and security validation
- [ ] Add session cleanup on completion and on error (Drop guard pattern)

### Task 12: Multi-Strategy Compaction (REQ-12)
- [ ] Implement `snip_stale_tool_results()` in `src/compact.rs`: identify large/old/failed tool results for removal, plus dedup of file read results (keep most recent per path)
- [ ] Implement `auto_compact()`: try snip first, fall back to summary if still over threshold
- [ ] Add `compaction_strategy` config option (summary/snip/auto)
- [ ] Wire auto strategy into the existing auto-compaction trigger

### Task 13: MCP OAuth (REQ-13)
- [ ] Add `keyring` as optional dependency behind `oauth` feature flag
- [ ] Create `src/mcp_auth.rs` with OAuth 2.0 + PKCE flow
- [ ] Implement auth server metadata discovery (`.well-known/oauth-authorization-server`)
- [ ] Implement browser-based authorization with localhost callback listener
- [ ] Implement token exchange, storage (keyring), and automatic refresh
- [ ] Add `oauth` field to `McpServerConfig`
- [ ] Wire OAuth into MCP client connection flow (both HTTP and stdio)

### Task 14: Tool Search (REQ-14)
- [ ] Create `src/tools/tool_search.rs` with case-insensitive keyword-based tool discovery
- [ ] Implement search across tool names and descriptions
- [ ] Add `tool_search_enabled` config option
- [ ] When enabled and tool count > 20, include only core tools (fs_read, fs_write, file_edit, execute_bash, glob, grep, tool_search) in system prompt, defer others
- [ ] Implement tool re-registration: after tool_search returns results, promote matching deferred tools to active set so LLM can call them in subsequent turns
- [ ] Register `"tool_search"` in `build_builtin_tools()`
