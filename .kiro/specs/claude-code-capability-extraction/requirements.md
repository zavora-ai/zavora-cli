# Requirements: Claude Code Capability Extraction for Zavora-CLI

## Context

Zavora-CLI is a Rust CLI AI agent built on ADK-Rust. Analysis of the Claude Code v2.1.88 source reveals production-grade patterns for tool safety, code intelligence, MCP infrastructure, and agent orchestration that would significantly improve zavora-cli's capabilities. This spec defines the requirements for extracting and adapting those patterns into zavora-cli's Rust codebase.

### Current State (Zavora-CLI v1.1.5)

- **Tools**: `fs_read`, `fs_write` (create/overwrite/append/patch modes), `execute_bash` (basic denied patterns + readonly command allowlist), `github_ops`, `todo_list`, `current_unix_time`, `release_template`
- **MCP**: HTTP-only client via `adk-tool::McpHttpClientBuilder`. Config: `name`, `endpoint`, `auth_bearer_env`, `tool_allowlist`, `tool_aliases`
- **Agents**: orchestrator, sequential, file_loop, memory, quality, search, time, ralph
- **Security**: `EXECUTE_BASH_DENIED_PATTERNS` (13 patterns), `READONLY_COMMANDS` (50+ commands), workspace path sandboxing on `fs_read`/`fs_write`
- **Compaction**: Manual `/compact` + ADK `EventsCompactionConfig` auto-compaction
- **Permissions**: `ToolConfirmationMode` (disabled/all/mcp-only), per-tool `approve_tool`/`require_confirm_tool` lists

### Reference (Claude Code v2.1.88)

- **BashSecurity**: ~25 validation checks across 6 files (command substitution, heredoc safety, shell metacharacters, IFS injection, brace expansion, unicode whitespace, obfuscated flags, proc/environ access, carriage return injection, backslash-escaped operators, comment-quote desync, quoted newlines, mid-word hash, Zsh-specific dangerous commands)
- **LSPTool**: 9 operations via Language Server Protocol (goToDefinition, findReferences, hover, documentSymbol, workspaceSymbol, goToImplementation, prepareCallHierarchy, incomingCalls, outgoingCalls)
- **FileEditTool**: Surgical `old_string → new_string` with `replace_all`, quote-style preservation, fuzzy matching fallback
- **GlobTool**: Structured file pattern search with `.gitignore` respect
- **GrepTool**: Ripgrep wrapper with context lines, output modes, multiline, offset/limit pagination
- **WebFetchTool**: HTTP fetch → HTML-to-markdown conversion → LLM prompt application
- **MCP**: 7 transports (stdio, SSE, HTTP, WebSocket, SDK, in-process, proxy), full OAuth 2.0 + XAA, server mode (expose tools as MCP server)
- **Permissions**: Layered system — validateInput → pre-tool hooks → pattern-based rules (always allow/deny/ask with glob matching) → interactive prompt → tool-specific checkPermissions
- **Parallel Execution**: `StreamingToolExecutor` partitions concurrent-safe vs serial tools
- **Sub-Agents**: Fork with fresh context, in-process teammates, worktree isolation

---

## REQ-1: MCP Server Mode

### REQ-1.1
Zavora-cli MUST expose itself as an MCP server over stdio transport, registering all built-in tools as MCP tools with JSON Schema input/output definitions.

### REQ-1.2
The MCP server MUST be invocable via `zavora mcp serve` CLI subcommand.

### REQ-1.3
The MCP server MUST implement the MCP `tools/list` and `tools/call` request handlers per the MCP specification.

### REQ-1.4
Tool call results MUST be returned as MCP `CallToolResult` with `content` array containing text blocks, and `isError: true` on failure.

### REQ-1.5
The MCP server MUST use the `@modelcontextprotocol/sdk` equivalent Rust crate (`rmcp` or similar) for protocol compliance.

### REQ-1.6
The MCP server MUST bypass interactive permission checks — the MCP client is treated as a trusted caller. Tool-level input validation (e.g., workspace path policy) MUST still be enforced.

### REQ-1.7
The MCP server MUST be stateless — each `tools/call` request is independent with no session context carried between calls.

---

## REQ-2: MCP Stdio Client Transport

### REQ-2.1
Zavora-cli's MCP client MUST support stdio transport — spawning a child process and communicating via stdin/stdout JSON-RPC.

### REQ-2.2
MCP server config MUST be extended with a `transport` field supporting `"stdio"` and `"http"` (default: inferred from config shape).

### REQ-2.3
Stdio config MUST accept `command` (string), `args` (string array), and optional `env` (string→string map).

### REQ-2.4
The stdio client MUST handle process lifecycle: spawn on connect, kill on disconnect, restart on crash with configurable retry.

### REQ-2.5
Existing HTTP-only `McpServerConfig` MUST remain backward-compatible. Configs without `transport` or `command` fields MUST default to HTTP behavior.

---

## REQ-3: String-Replace File Edit Tool

### REQ-3.1
A new `file_edit` tool MUST be added that performs surgical text replacement: given `file_path`, `old_string`, `new_string`, and optional `replace_all` (default false).

### REQ-3.2
The tool MUST fail if `old_string` is not found in the file, returning the closest match with a similarity hint.

### REQ-3.3
The tool MUST fail if `old_string` matches multiple locations and `replace_all` is false, reporting the number of matches.

### REQ-3.4
The tool MUST preserve the file's original line endings (LF vs CRLF).

### REQ-3.5
The tool MUST enforce the same workspace path policy as `fs_write` (no writes outside workspace root, no writes to denied segments).

### REQ-3.6
The tool MUST return a structured diff (unified format) showing the change.

### REQ-3.7
The existing `fs_write` patch mode SHOULD be preserved for backward compatibility but the new `file_edit` tool SHOULD be the primary edit tool in the system prompt.

### REQ-3.8
The tool MUST reject files larger than 10MB to prevent out-of-memory conditions.

### REQ-3.9
If `old_string` equals `new_string`, the tool MUST return an error indicating no change is needed.

---

## REQ-4: Glob Tool

### REQ-4.1
A new `glob` tool MUST be added that finds files matching a glob pattern within a directory.

### REQ-4.2
The tool MUST respect `.gitignore` rules when searching.

### REQ-4.3
The tool MUST return structured output: `{ numFiles, filenames, truncated, durationMs }`.

### REQ-4.4
Results MUST be truncated at a configurable limit (default 100 files) with `truncated: true` indicator.

### REQ-4.5
The tool MUST accept an optional `path` parameter for the search root directory (default: cwd).

### REQ-4.6
The tool MUST enforce workspace path policy — no glob searches outside the workspace root.

---

## REQ-5: Grep Tool

### REQ-5.1
A new `grep` tool MUST be added that searches file contents using regex patterns, wrapping `ripgrep` (`rg`).

### REQ-5.2
The tool MUST support these parameters: `pattern` (regex), `path` (optional search root), `glob` (file filter), `output_mode` (content/files_with_matches/count), context lines (`-B`, `-A`, `-C`), case-insensitive (`-i`), `head_limit` (default 250), `offset`, `multiline`.

### REQ-5.3
The tool MUST exclude VCS directories (`.git`, `.svn`, `.hg`) automatically.

### REQ-5.4
The tool MUST return structured output with match counts, file counts, duration, and truncation status.

### REQ-5.5
The tool MUST gracefully handle missing `rg` binary by falling back to `grep -rn` or returning an actionable error.

### REQ-5.6
The tool MUST enforce workspace path policy.

---

## REQ-6: Bash Security Validation Layer

### REQ-6.1
The `execute_bash` tool MUST implement a multi-stage security validation pipeline that runs BEFORE permission checks, replacing the current flat denied-patterns list.

### REQ-6.2
The validation pipeline MUST include these checks (each returning allow/deny/ask/passthrough):

1. **Empty command** — allow empty/whitespace-only commands
2. **Incomplete commands** — deny commands starting with tab, flags, or continuation operators (`&&`, `||`, `;`, `>`, `<`)
3. **Command substitution** — deny unquoted `$()`, `` ` ``, `<()`, `>()`, `${}`, `$[]` outside of safe heredoc patterns
4. **Shell metacharacters** — deny unquoted pipe `|`, backgrounding `&`, semicolons `;` in non-trivial positions
5. **Dangerous variables** — deny `IFS=`, `PATH=`, `LD_PRELOAD=`, `LD_LIBRARY_PATH=` assignments
6. **Newline injection** — deny literal newlines in commands (multi-line commands must use heredocs or explicit continuation)
7. **Redirection validation** — deny output redirections except to `/dev/null`, stderr-to-stdout (`2>&1`), and stdin from `/dev/null`
8. **Heredoc safety** — allow only single-quoted or escaped-delimiter heredocs in `$(cat <<'DELIM' ...)` form; reject heredocs with unquoted delimiters (which allow variable expansion)
9. **Obfuscated flags** — deny flag-like arguments containing shell metacharacters or non-ASCII characters
10. **Brace expansion** — deny `{a,b}` and `{1..10}` patterns outside of quoted strings
11. **Unicode whitespace** — deny non-ASCII whitespace characters (U+00A0, U+2000–U+200F, etc.) that can hide content
12. **Carriage return injection** — deny `\r` characters that can mask command content in terminal display
13. **Proc/environ access** — deny reads from `/proc/*/environ`, `/proc/*/cmdline`, and similar sensitive procfs paths
14. **IFS injection** — deny `IFS` variable manipulation
15. **Backslash-escaped operators** — deny `\|`, `\&`, `\;` that may bypass naive pattern matching
16. **Comment-quote desync** — deny patterns where `#` appears adjacent to closing quotes (e.g., `'x'#`) which can cause parser confusion
17. **Zsh dangerous commands** — deny `zmodload`, `emulate`, `sysopen`, `syswrite`, `zpty`, `ztcp`, and other Zsh-specific dangerous builtins
18. **Mid-word hash** — deny `#` appearing mid-word (e.g., `foo#bar`) outside of quoted strings, which can cause shell parser confusion between comments and parameter expansion
19. **Malformed token injection** — deny tokens that exploit shell parser bugs through malformed quoting or escape sequences
20. **jq system function** — deny `jq` commands containing the `system()` or `@sh` functions which can execute arbitrary shell commands
21. **Git commit substitution** — deny `git commit -m` with messages containing `$()` or backtick substitution that would be expanded by the shell

### REQ-6.3
Each validation check MUST be an independent function returning a `SecurityResult` enum (`Allow`, `Deny(reason)`, `Ask(reason)`, `Passthrough`).

### REQ-6.4
The pipeline MUST short-circuit on the first `Deny` or `Allow` result. `Ask` results MUST be collected and presented to the user. `Passthrough` continues to the next check.

### REQ-6.5
The validation pipeline MUST operate on both the raw command string AND a quote-extracted version (content with single/double quotes stripped) to catch obfuscation.

### REQ-6.6
The existing `READONLY_COMMANDS` allowlist MUST be preserved as an early-exit auto-approve path before the security pipeline runs.

### REQ-6.7
A `validate_bash_command(command: &str) -> SecurityResult` public function MUST be exposed for use by both the tool and the permission system.

---

## REQ-7: LSP Tool (Language Server Protocol)

### REQ-7.1
A new `lsp` tool MUST be added that provides semantic code intelligence by communicating with language servers.

### REQ-7.2
The tool MUST support these operations:
- `goToDefinition` — find where a symbol is defined
- `findReferences` — find all usages of a symbol
- `hover` — get type/documentation info for a symbol
- `documentSymbol` — list all symbols in a file
- `workspaceSymbol` — search symbols across the workspace
- `goToImplementation` — find implementations of interfaces/traits
- `prepareCallHierarchy` — get call hierarchy item at position
- `incomingCalls` — find callers of a function
- `outgoingCalls` — find callees of a function

### REQ-7.3
The tool input MUST accept: `operation` (enum), `filePath` (string), `line` (1-based integer), `character` (1-based integer).

### REQ-7.4
The tool MUST manage LSP server lifecycle: start on first use, keep alive during session, shutdown on exit.

### REQ-7.5
The tool MUST support at minimum these language servers:
- `rust-analyzer` for Rust
- `typescript-language-server` for TypeScript/JavaScript
- `pylsp` or `pyright` for Python
- `gopls` for Go

### REQ-7.6
LSP server configuration MUST be stored in `.kiro/settings/lsp.json` (or `.zavora/lsp.json`) with per-language server command and args.

### REQ-7.7
The tool MUST auto-detect file language from extension and route to the appropriate server.

### REQ-7.8
The tool MUST handle the two-step call hierarchy flow: first `prepareCallHierarchy` to get a `CallHierarchyItem`, then `incomingCalls`/`outgoingCalls` using that item.

### REQ-7.9
Results MUST be formatted as human-readable text with file paths relative to cwd, line numbers, and symbol kinds.

### REQ-7.10
The tool MUST filter results through `.gitignore` to exclude generated/vendored files.

### REQ-7.11
The tool MUST be gated behind an opt-in flag or initialization command (e.g., `zavora lsp init`) since language servers are heavy dependencies.

### REQ-7.12
The tool MUST validate that the target file exists and is under the 10MB size limit before sending LSP requests.

### REQ-7.13
When any write tool (`file_edit`, `fs_write`, `execute_bash` writing to files) modifies a file that has been opened in an LSP server, the LSP manager MUST send a `textDocument/didChange` notification to keep the server in sync. Failure to sync MUST NOT block the write operation.

### REQ-7.14
The LSP manager MUST handle language server crashes gracefully — auto-restart on the next tool call for that language, with a maximum of 3 restart attempts per session.

---

## REQ-8: Web Fetch Tool

### REQ-8.1
A new `web_fetch` tool MUST be added that fetches a URL and returns its content as markdown.

### REQ-8.2
The tool MUST convert HTML to markdown using a library like `htmd` or `html2text`.

### REQ-8.3
The tool MUST accept `url` (required) and `prompt` (required — instruction for processing the fetched content).

### REQ-8.4
The tool MUST enforce a domain blocklist for known-dangerous or SSRF-prone domains (localhost, 169.254.x.x metadata endpoints, internal IPs).

### REQ-8.5
The tool MUST follow redirects (up to 5 hops) but only to permitted domains.

### REQ-8.6
The tool MUST truncate content to a configurable maximum (default 100KB of markdown).

### REQ-8.7
The tool MUST return structured output: `{ url, code, codeText, bytes, result, durationMs }`.

### REQ-8.8
The tool MUST require permission confirmation (not auto-approved) since it makes network requests.

---

## REQ-9: Layered Permission System

### REQ-9.1
The permission system MUST be restructured into a pipeline: `validateInput()` → pre-tool hooks → permission rules → interactive prompt → tool-specific `checkPermissions()`.

### REQ-9.2
Permission rules MUST support glob/wildcard patterns for tool names and content matching:
```toml
always_allow = ["fs_read:*", "grep:*", "glob:*", "execute_bash:git status*"]
always_deny = ["execute_bash:rm -rf *", "fs_write:/etc/*"]
always_ask = ["web_fetch:*", "github_ops:*"]
```

### REQ-9.3
Rules MUST be configurable at three levels: profile config, agent config, and session-level via `/allow <pattern>` and `/deny <pattern>` slash commands that modify rules for the current session only.

### REQ-9.4
Each tool MUST declare `is_read_only()` and `is_concurrency_safe()` trait methods that the permission system uses for default behavior.

### REQ-9.5
Read-only tools MUST be auto-approved by default (no confirmation needed).

### REQ-9.6
The existing `ToolConfirmationMode` and `approve_tool`/`require_confirm_tool` config MUST remain backward-compatible, mapped into the new rule system.

---

## REQ-10: Parallel Tool Execution

### REQ-10.1
When the LLM returns multiple tool calls in a single response, the runner MUST partition them into concurrent-safe and serial groups.

### REQ-10.2
Tools that declare `is_concurrency_safe() = true` AND `is_read_only() = true` MUST be executed in parallel using `tokio::join!` or `futures::join_all`.

### REQ-10.3
Tools that are NOT concurrency-safe MUST be executed sequentially in the order returned by the LLM.

### REQ-10.4
Mixed batches MUST execute all concurrent-safe tools first (in parallel), then serial tools sequentially.

### REQ-10.5
Tool execution errors in parallel batches MUST NOT abort other parallel tools — each result is collected independently.

---

## REQ-11: Fork Sub-Agents with Fresh Context

### REQ-11.1
The `AgentTool` (or equivalent delegation mechanism) MUST support spawning sub-agents with a fresh message history, preventing context pollution from the parent conversation.

### REQ-11.2
The sub-agent MUST receive only: the task prompt, relevant file context (if any), and the parent's tool set.

### REQ-11.3
The sub-agent result MUST be returned to the parent as a single tool result message containing the sub-agent's final output.

### REQ-11.4
Sub-agent execution MUST respect the same permission rules and security validation as the parent.

### REQ-11.5
Sub-agent sessions MUST be cleaned up after completion to prevent session storage bloat.

---

## REQ-12: Multi-Strategy Compaction

### REQ-12.1
The compaction system MUST support three strategies:
1. **Summary compaction** (existing) — LLM-generated summary of older events
2. **Snip compaction** — remove stale tool results (large outputs from completed operations) without summarizing
3. **Threshold-triggered auto-compaction** — automatically compact when context utilization exceeds a configurable threshold (default 75%)

### REQ-12.2
Snip compaction MUST identify and remove: tool results larger than 2KB that are more than N events old, duplicate file read results (keep only the most recent), and failed tool results.

### REQ-12.3
The compaction strategy MUST be configurable: `compaction_strategy = "summary" | "snip" | "auto"` (default: `"auto"` which uses snip first, then summary if still over threshold).

---

## REQ-13: MCP OAuth 2.0 Authentication

### REQ-13.1
MCP server config MUST support an `oauth` field with: `client_id` (optional), `callback_port` (optional), `auth_server_metadata_url` (optional).

### REQ-13.2
The OAuth flow MUST implement Authorization Code with PKCE: open browser for authorization, listen on localhost callback, exchange code for tokens.

### REQ-13.3
Tokens MUST be persisted securely (OS keychain via `keyring` crate, or encrypted file fallback) and refreshed automatically on expiry.

### REQ-13.4
The existing `auth_bearer_env` config MUST remain supported alongside OAuth.

---

## REQ-14: Tool Search / Dynamic Tool Loading

### REQ-14.1
A `tool_search` tool MUST be added that lets the LLM discover available tools by keyword search, rather than loading all tool descriptions into the system prompt.

### REQ-14.2
When tool search is enabled, only a subset of core tools (fs_read, fs_write, file_edit, execute_bash, glob, grep, tool_search) MUST be included in the initial system prompt. Other tools are deferred.

### REQ-14.3
The tool_search tool MUST search tool names and descriptions, returning matching tool schemas so the LLM can use them in subsequent turns.

### REQ-14.4
Tool search MUST be opt-in via config (`tool_search_enabled = true`) and only activated when the total tool count exceeds a threshold (default 20).

---

## Priority and Phasing

### Phase 1 — Foundation (High value, contained effort)
- REQ-3: String-Replace File Edit Tool
- REQ-4: Glob Tool
- REQ-5: Grep Tool
- REQ-8: Web Fetch Tool

### Phase 2 — MCP Infrastructure
- REQ-1: MCP Server Mode
- REQ-2: MCP Stdio Client Transport
- REQ-9: Layered Permission System

### Phase 3 — Security & Intelligence
- REQ-6: Bash Security Validation Layer
- REQ-7: LSP Tool

### Phase 4 — Advanced Patterns
- REQ-10: Parallel Tool Execution
- REQ-11: Fork Sub-Agents
- REQ-12: Multi-Strategy Compaction
- REQ-13: MCP OAuth
- REQ-14: Tool Search
