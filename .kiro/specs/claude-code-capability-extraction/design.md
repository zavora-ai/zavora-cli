# Design: Claude Code Capability Extraction for Zavora-CLI

## Overview

This document specifies the technical design for extracting and adapting production patterns from Claude Code v2.1.88 into zavora-cli's Rust codebase. Each section maps to a requirement group and defines file locations, data structures, interfaces, and implementation approach.

---

## D-1: MCP Server Mode (REQ-1)

### Architecture

Zavora-cli exposes its built-in tools as an MCP server over stdio. A new binary entrypoint or subcommand starts the server, which reads JSON-RPC from stdin and writes to stdout.

### New Files

- `src/mcp_server.rs` — MCP server implementation

### Data Structures

```rust
// No new config types needed — reuses existing tool registry

struct McpServerState {
    tools: Vec<Arc<dyn Tool>>,
    workspace_root: PathBuf,
}
```

### Protocol Flow

```
stdin → JSON-RPC → match method {
    "tools/list"  → iterate tools, emit name + JSON Schema from tool.parameters_schema()
    "tools/call"  → find tool by name, deserialize args, call tool.call(ctx, args), serialize result
    "initialize"  → return server capabilities { tools: {} }
}
→ JSON-RPC → stdout
```

### Implementation

Use the `rmcp` crate (Rust MCP SDK) or implement minimal JSON-RPC over stdio directly. The server wraps `build_builtin_tools()` and converts each ADK `Tool` trait object into an MCP tool definition by extracting its name, description, and parameter schema.

### CLI Integration

```rust
// In cli.rs, add to McpCommands:
Serve => run_mcp_server(cfg).await
```

### Key Decisions

- Stdio only (no HTTP server mode for MCP — that's what `zavora server serve` already does for A2A)
- All tools registered, permission checks bypassed (MCP client is trusted) — tool-level input validation (workspace path policy) still enforced
- No session state — each tool call is stateless
- Uses Content-Length framed JSON-RPC (same as LSP), NOT line-delimited — shares framing implementation with LSP client (see D-7)

---

## D-2: MCP Stdio Client Transport (REQ-2)

### Architecture

Extend `McpServerConfig` to support stdio transport alongside existing HTTP. The stdio client spawns a child process and communicates via stdin/stdout JSON-RPC.

### Modified Files

- `src/config.rs` — extend `McpServerConfig`
- `src/mcp.rs` — add stdio client logic

### Data Structures

```rust
#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(tag = "transport", rename_all = "lowercase")]
pub enum McpTransport {
    Http {
        endpoint: String,
        #[serde(default)]
        headers: HashMap<String, String>,
    },
    Stdio {
        command: String,
        #[serde(default)]
        args: Vec<String>,
        #[serde(default)]
        env: HashMap<String, String>,
    },
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct McpServerConfig {
    pub name: String,
    #[serde(flatten)]
    pub transport: McpTransport,
    pub enabled: Option<bool>,
    pub timeout_secs: Option<u64>,
    pub auth_bearer_env: Option<String>,
    #[serde(default)]
    pub tool_allowlist: Vec<String>,
    #[serde(default)]
    pub tool_aliases: HashMap<String, String>,
    // OAuth (Phase 4)
    pub oauth: Option<McpOAuthConfig>,
}
```

### Backward Compatibility

Configs with bare `endpoint` field (no `transport` tag) deserialize as `McpTransport::Http { endpoint }` via a custom deserializer or `#[serde(untagged)]` fallback.

### Stdio Client Flow

```
1. spawn child process (command + args + env)
2. write JSON-RPC "initialize" to child stdin
3. read JSON-RPC response from child stdout
4. write "tools/list" → parse tool definitions
5. for each tool call: write "tools/call" → read result
6. on disconnect: send SIGTERM, wait 5s, SIGKILL
```

### Process Management

- Spawn via `tokio::process::Command`
- Content-Length framed JSON-RPC (`Content-Length: N\r\n\r\n{json}`) — same framing as LSP protocol. Shares the `JsonRpcFraming` implementation with the LSP client (D-7).
- Reconnect on crash with exponential backoff (500ms → 30s, max 5 attempts)

---

## D-3: String-Replace File Edit Tool (REQ-3)

### New Files

- `src/tools/file_edit.rs`

### Data Structures

```rust
pub struct FileEditRequest {
    pub file_path: String,
    pub old_string: String,
    pub new_string: String,
    pub replace_all: bool,
}
```

### Algorithm

```
1. resolve + validate path (reuse fs_read workspace policy)
2. read file content — reject if file > 10MB
3. if old_string == new_string → return error "no change needed"
4. count occurrences of old_string in content
5. if count == 0 → find closest match via strsim::normalized_levenshtein, return error with hint
6. if count > 1 && !replace_all → return error with count
7. perform replacement (str::replace or str::replacen(1))
8. write file (preserve original line endings)
9. compute unified diff (similar::TextDiff)
10. return { filePath, diff, replacements_made }
```

### Line Ending Preservation

Detect line endings before edit by scanning first 1KB for `\r\n` vs `\n`. After replacement, normalize to the detected style.

### Integration

Register as `"file_edit"` in `build_builtin_tools()`. Update system prompt to prefer `file_edit` over `fs_write` patch mode for surgical edits.

---

## D-4: Glob Tool (REQ-4)

### New Files

- `src/tools/glob.rs`

### Implementation

Use the `ignore` crate (same crate ripgrep uses) which provides both `.gitignore`-aware directory walking and built-in glob matching via its `globset` module. No separate `glob` crate needed.

```rust
pub struct GlobRequest {
    pub pattern: String,
    pub path: Option<String>,  // default: cwd
}

pub struct GlobOutput {
    pub num_files: usize,
    pub filenames: Vec<String>,  // relative to cwd
    pub truncated: bool,
    pub duration_ms: u64,
}
```

### Key Behavior

- Walk directory using `ignore::WalkBuilder` (respects `.gitignore`)
- Match each entry against the glob pattern
- Truncate at 100 files
- Return paths relative to cwd
- Enforce workspace path policy on the search root

---

## D-5: Grep Tool (REQ-5)

### New Files

- `src/tools/grep.rs`

### Implementation

Shell out to `rg` (ripgrep) with structured arguments. Parse output into structured result.

```rust
pub struct GrepRequest {
    pub pattern: String,
    pub path: Option<String>,
    pub glob: Option<String>,
    pub file_type: Option<String>,    // rg --type (e.g., "rust", "py", "js")
    pub output_mode: Option<GrepOutputMode>,  // content | files_with_matches | count
    pub context_before: Option<usize>,
    pub context_after: Option<usize>,
    pub context: Option<usize>,
    pub case_insensitive: Option<bool>,
    pub head_limit: Option<usize>,  // default 250
    pub offset: Option<usize>,
    pub multiline: Option<bool>,
}

pub enum GrepOutputMode { Content, FilesWithMatches, Count }

pub struct GrepOutput {
    pub num_files: usize,
    pub num_matches: usize,
    pub results: Vec<GrepFileResult>,  // per-file results
    pub truncated: bool,
    pub duration_ms: u64,
}

pub struct GrepFileResult {
    pub file: String,
    pub count: usize,
    pub matches: Vec<String>,  // matched lines (content mode only)
}
```

### Ripgrep Invocation

Build `rg` command args from request fields. Always add `--glob '!.git'`. Parse stdout, apply offset + head_limit, return structured output.

### Fallback

If `rg` is not found in PATH, fall back to `grep -rn --include=<glob>` with reduced feature set (no multiline, no offset).

---

## D-6: Bash Security Validation Layer (REQ-6)

### New Files

- `src/tools/bash_security.rs`

### Architecture

A pipeline of independent validation functions, each receiving a `ValidationContext` and returning a `SecurityResult`.

```rust
pub enum SecurityResult {
    Allow { reason: String },
    Deny { reason: String },
    Ask { reason: String },
    Passthrough,
}

pub struct ValidationContext {
    pub original_command: String,
    pub base_command: String,              // first word
    pub unquoted_content: String,          // content with single quotes stripped
    pub fully_unquoted: String,            // content with all quotes stripped, safe redirections stripped
    pub fully_unquoted_pre_strip: String,  // all quotes stripped, BEFORE safe-redirection stripping
                                           // (needed by validate_brace_expansion to avoid false adjacencies)
    pub unquoted_keep_quote_chars: String, // strips quoted CONTENT but keeps quote delimiters ('/"")
                                           // (needed by validate_comment_quote_desync to detect 'x'# patterns)
}
```

### Quote Extraction

Port the `extractQuotedContent()` logic: iterate chars tracking `in_single_quote` / `in_double_quote` / `escaped` state, producing four stripped versions:
1. `unquoted_content` — single-quote content removed
2. `fully_unquoted` — all quote content removed, then safe redirections stripped via `strip_safe_redirections()`
3. `fully_unquoted_pre_strip` — all quote content removed, BEFORE safe-redirection stripping
4. `unquoted_keep_quote_chars` — quote content removed but delimiter characters (`'`, `"`) preserved

### Safe Redirection Stripping

```rust
fn strip_safe_redirections(content: &str) -> String {
    // Remove: `2>&1`, `N>/dev/null`, `</dev/null`
    // SECURITY: Each pattern MUST have a trailing boundary (\s|$)
    // to prevent prefix matching (e.g., "> /dev/nullo" must NOT match)
}
```

### Validation Pipeline

```rust
pub fn validate_bash_command(command: &str) -> SecurityResult {
    let ctx = build_validation_context(command);

    let checks: &[fn(&ValidationContext) -> SecurityResult] = &[
        validate_empty,
        validate_incomplete_commands,
        validate_zsh_dangerous_commands,
        validate_command_substitution,
        validate_shell_metacharacters,
        validate_dangerous_variables,
        validate_newlines,
        validate_redirections,
        validate_heredoc_safety,
        validate_obfuscated_flags,
        validate_brace_expansion,
        validate_unicode_whitespace,
        validate_carriage_return,
        validate_proc_environ_access,
        validate_ifs_injection,
        validate_backslash_escaped_operators,
        validate_comment_quote_desync,
        validate_mid_word_hash,
        validate_malformed_token_injection,
        validate_jq_system_function,
        validate_git_commit_substitution,
    ];

    let mut ask_reasons = Vec::new();
    for check in checks {
        match check(&ctx) {
            SecurityResult::Allow { .. } => return SecurityResult::Allow { .. },
            SecurityResult::Deny { reason } => return SecurityResult::Deny { reason },
            SecurityResult::Ask { reason } => ask_reasons.push(reason),
            SecurityResult::Passthrough => continue,
        }
    }

    if !ask_reasons.is_empty() {
        return SecurityResult::Ask { reason: ask_reasons.join("; ") };
    }
    SecurityResult::Passthrough
}
```

### Integration with execute_bash.rs

Replace the current flat `DENIED_PATTERNS` check with:

```rust
// In execute_bash tool handler:
match validate_bash_command(&command) {
    SecurityResult::Deny { reason } => return error_payload(&reason),
    SecurityResult::Ask { reason } => { /* route to confirmation flow */ },
    _ => { /* proceed */ }
}
```

### Key Validation Details

**Command substitution** — Check `fully_unquoted` for `$(`, `` ` ``, `<(`, `>(`, `${`, `$[`. Exception: safe heredoc patterns `$(cat <<'DELIM' ... DELIM)`.

**Shell metacharacters** — Check `fully_unquoted` for unescaped `|`, `&`, `;`. Exception: `2>&1` and `> /dev/null`.

**Dangerous variables** — Regex match `\b(IFS|PATH|LD_PRELOAD|LD_LIBRARY_PATH)\s*=` in `fully_unquoted`.

**Unicode whitespace** — Check for any char in `[\u00A0\u1680\u2000-\u200F\u2028\u2029\u202F\u205F\u3000\uFEFF]`.

**Brace expansion** — Match `\{[^}]*,[^}]*\}` or `\{[0-9]+\.\.[0-9]+\}` in `fully_unquoted_pre_strip` (use pre-strip to avoid false adjacencies from redirection removal).

**Mid-word hash** — Check `unquoted_keep_quote_chars` for `#` adjacent to non-whitespace (e.g., `'x'#`, `foo#bar`). The quote chars must be visible to detect quote-adjacent patterns.

**jq system function** — If base command is `jq`, check for `system`, `@sh`, `input`, `debug` in the jq expression argument.

**Git commit substitution** — If command matches `git commit -m`, check the message argument for `$()` or backtick substitution in `unquoted_content`.

---

## D-7: LSP Tool (REQ-7)

### New Files

- `src/lsp/mod.rs` — module root
- `src/lsp/manager.rs` — server lifecycle management
- `src/lsp/client.rs` — LSP JSON-RPC client
- `src/tools/lsp.rs` — tool implementation

### Architecture

```
LspManager
  ├── servers: HashMap<Language, LspServerHandle>
  ├── start_server(language) → spawn process, initialize
  ├── send_request(file, method, params) → route to correct server
  └── shutdown_all()

LspServerHandle
  ├── process: Child
  ├── stdin: ChildStdin (write requests)
  ├── stdout_reader: task reading responses
  ├── pending_requests: HashMap<RequestId, oneshot::Sender>
  └── open_files: HashSet<PathBuf>
```

### LSP JSON-RPC Protocol

Each message has a `Content-Length: N\r\n\r\n` header followed by JSON body. Use `lsp-types` crate for type definitions (NOT `tower-lsp` which is a server library). The Content-Length framing implementation is shared with the MCP stdio client (D-2) via a common `JsonRpcFraming` module.

### Language Detection

```rust
fn detect_language(path: &Path) -> Option<Language> {
    match path.extension()?.to_str()? {
        "rs" => Some(Language::Rust),
        "ts" | "tsx" | "js" | "jsx" => Some(Language::TypeScript),
        "py" => Some(Language::Python),
        "go" => Some(Language::Go),
        "java" => Some(Language::Java),
        "rb" => Some(Language::Ruby),
        "c" | "h" | "cpp" | "hpp" | "cc" => Some(Language::Cpp),
        _ => None,
    }
}
```

### Server Config (`.zavora/lsp.json`)

```json
{
  "servers": {
    "rust": { "command": "rust-analyzer", "args": [] },
    "typescript": { "command": "typescript-language-server", "args": ["--stdio"] },
    "python": { "command": "pylsp", "args": [] },
    "go": { "command": "gopls", "args": ["serve"] },
    "java": { "command": "jdtls", "args": [] },
    "ruby": { "command": "solargraph", "args": ["stdio"] },
    "cpp": { "command": "clangd", "args": [] }
  }
}
```

Note: `zavora lsp init` generates config only for servers found in PATH. Users can add others manually.

### Tool Implementation

```rust
// Input
pub struct LspRequest {
    pub operation: LspOperation,
    pub file_path: String,
    pub line: u32,      // 1-based
    pub character: u32,  // 1-based
}

pub enum LspOperation {
    GoToDefinition, FindReferences, Hover,
    DocumentSymbol, WorkspaceSymbol, GoToImplementation,
    PrepareCallHierarchy, IncomingCalls, OutgoingCalls,
}
```

### Call Hierarchy Two-Step

For `incomingCalls`/`outgoingCalls`:
1. Send `textDocument/prepareCallHierarchy` → get `CallHierarchyItem[]`
2. Send `callHierarchy/incomingCalls` or `callHierarchy/outgoingCalls` with `{ item: items[0] }`

### Result Formatting

Format results as human-readable text:
```
Found 3 references in 2 files:
  src/main.rs:42:10 - let x = foo();
  src/lib.rs:15:5  - pub fn foo() {
  src/lib.rs:88:12 - foo()
```

### Lifecycle

- Lazy start: server starts on first tool call for that language
- Keep alive: server persists for the session duration
- Crash recovery: on server crash, mark as dead; auto-restart on next tool call (max 3 restarts per session per language)
- Shutdown: `shutdown` request + `exit` notification on session end
- File sync: `textDocument/didOpen` before first request per file
- Edit sync: `textDocument/didChange` when write tools modify an open file (called by file_edit/fs_write hooks)
- File close: `textDocument/didClose` when session ends (batch close all open files before shutdown)
- Stderr: LSP server stderr is logged at `tracing::debug` level for diagnostics

---

## D-8: Web Fetch Tool (REQ-8)

### New Files

- `src/tools/web_fetch.rs`

### Implementation

Use `reqwest` for HTTP and `htmd` or `html2text` for HTML→markdown conversion.

```rust
pub struct WebFetchRequest {
    pub url: String,
    pub prompt: String,
}

pub struct WebFetchOutput {
    pub url: String,
    pub code: u16,
    pub code_text: String,
    pub bytes: usize,
    pub result: String,
    pub duration_ms: u64,
}
```

### Domain Blocklist

```rust
const BLOCKED_DOMAINS: &[&str] = &[
    "localhost", "127.0.0.1", "0.0.0.0", "::1",
    "169.254.169.254",  // AWS metadata
    "metadata.google.internal",  // GCP metadata
];

fn is_blocked_domain(url: &str) -> bool {
    // Parse URL, check host against blocklist
    // Also block private IP ranges: 10.x, 172.16-31.x, 192.168.x
}
```

### Flow

```
1. validate URL format
2. check domain blocklist (check each redirect hop too)
3. HTTP GET with timeout (30s), follow redirects (max 5), User-Agent: "zavora-cli/<version>"
4. check Content-Type — if HTML, convert to markdown; if JSON, pretty-print; if text, use raw
5. truncate to 100KB
6. the `prompt` parameter is returned alongside the content as context for the LLM —
   the tool does NOT make an LLM call itself; the calling agent applies the prompt
   to the fetched content in its next reasoning step
7. return structured output
```

### Dependencies

Add to `Cargo.toml`:
```toml
reqwest = { version = "0.12", features = ["rustls-tls"] }
htmd = "0.1"  # or html2text = "0.12"
```

---

## D-9: Layered Permission System (REQ-9)

### Modified Files

- `src/tools/confirming.rs` — refactor into pipeline
- `src/tool_policy.rs` — add pattern matching

### Permission Rule Structure

```rust
pub struct PermissionRules {
    pub always_allow: Vec<ToolPattern>,
    pub always_deny: Vec<ToolPattern>,
    pub always_ask: Vec<ToolPattern>,
}

pub struct ToolPattern {
    pub tool_name: String,     // glob: "execute_bash", "*"
    pub content_pattern: Option<String>,  // glob: "git status*", "/etc/*"
}
```

### Pipeline

```rust
pub async fn check_tool_permission(
    tool: &dyn Tool,
    args: &Value,
    rules: &PermissionRules,
    hooks: &[HookConfig],
) -> PermissionDecision {
    // 1. Validate input
    if let Err(e) = tool.validate_input(args) { return Deny(e); }

    // 2. Pre-tool hooks
    for hook in hooks { /* run shell command, check exit code */ }

    // 3. Pattern rules (first match wins)
    if let Some(rule) = rules.match_tool(tool.name(), args) {
        return rule.decision();
    }

    // 4. Default: read-only tools auto-approve, others ask
    if tool.is_read_only() { return Allow; }

    // 5. Tool-specific permission check (e.g., workspace path validation)
    if let Some(decision) = tool.check_permissions(args) { return decision; }

    return Ask;
}
```

### Session-Level Rules

Slash commands `/allow <pattern>` and `/deny <pattern>` modify a session-local `PermissionRules` overlay that takes precedence over profile/agent rules. These are not persisted — they reset on session end.

---

## D-10: Parallel Tool Execution (REQ-10)

### Modified Files

- `src/streaming.rs` or `src/runner.rs`

### Implementation

When processing a batch of tool calls from the LLM response, identify contiguous runs of concurrent-safe read-only tools and execute each run in parallel, preserving the original ordering for result assembly:

```rust
// Partition into contiguous groups preserving original order
let groups = group_by_concurrency(tool_calls);
// groups: [Parallel([read_A, read_B]), Serial(write_C), Parallel([read_D])]

let mut all_results = Vec::with_capacity(tool_calls.len());
for group in groups {
    match group {
        Group::Parallel(calls) => {
            // Execute concurrently, max 10 parallel
            let results = futures::future::join_all(
                calls.iter().map(|tc| execute_tool(tc))
            ).await;
            all_results.extend(results);
        }
        Group::Serial(call) => {
            all_results.push(execute_tool(&call).await);
        }
    }
}
// all_results is in the same order as the original tool_calls
```

This preserves the LLM's intended execution order while parallelizing contiguous read-only blocks. A maximum of 10 concurrent tool executions prevents resource exhaustion.

### Tool Trait Extension

Add to the ADK Tool trait (or wrapper):
```rust
fn is_concurrency_safe(&self) -> bool { false }  // default
fn is_read_only(&self) -> bool { false }          // default
```

Tools that are safe: `fs_read`, `glob`, `grep`, `lsp`, `current_unix_time`, `todo_list` (read ops).

---

## D-11: Fork Sub-Agents (REQ-11)

### Modified Files

- `src/agents/orchestrator.rs`
- `src/runner.rs`

### Implementation

Create a new session for the sub-agent, run the task, extract the result, clean up.

```rust
pub async fn fork_sub_agent(
    cfg: &RuntimeConfig,
    task_prompt: &str,
    file_context: Option<&str>,  // optional file content to include
    tools: Vec<Arc<dyn Tool>>,
    timeout: Duration,           // max execution time (default: 5 min)
) -> Result<String> {
    let sub_session_id = format!("fork-{}-{}", cfg.session_id, chrono::Utc::now().timestamp_millis());
    let mut sub_cfg = cfg.clone();
    sub_cfg.session_id = sub_session_id.clone();

    let full_prompt = match file_context {
        Some(ctx) => format!("{}\n\n<context>\n{}\n</context>", task_prompt, ctx),
        None => task_prompt.to_string(),
    };

    let agent = build_single_agent_with_tools(model, &tools, ...)?;
    let runner = build_runner(agent, &sub_cfg).await?;
    let result = tokio::time::timeout(
        timeout,
        run_prompt(&runner, &sub_cfg, &full_prompt),
    ).await??;

    // Cleanup (always, even on error — use Drop guard or finally pattern)
    let _ = session_service.delete(sub_session_id).await;
    Ok(result)
}
```

Sub-agent uses the same model as the parent by default. The model can be overridden via agent config.

---

## D-12: Multi-Strategy Compaction (REQ-12)

### Modified Files

- `src/compact.rs`

### Snip Strategy

```rust
pub fn snip_stale_tool_results(events: &[Event], max_age_events: usize) -> Vec<usize> {
    let mut to_remove = Vec::new();
    let mut seen_file_reads: HashMap<String, usize> = HashMap::new(); // path → most recent index

    for (i, e) in events.iter().enumerate() {
        let age = events.len() - i;
        let is_tool_result = e.author == "tool";

        if !is_tool_result { continue; }

        let text = extract_event_text(e);

        // Dedup: for file read results, keep only the most recent per path
        if let Some(path) = extract_file_read_path(e) {
            if let Some(prev_idx) = seen_file_reads.insert(path, i) {
                to_remove.push(prev_idx);
            }
        }

        // Snip: large or failed results older than threshold
        let is_large = text.len() > 2048;
        let is_failed = e.llm_response.content.is_none();
        if age > max_age_events && (is_large || is_failed) {
            to_remove.push(i);
        }
    }

    to_remove.sort();
    to_remove.dedup();
    to_remove
}
```

### Auto Strategy

```rust
pub async fn auto_compact(session_service, cfg) -> Result<String> {
    // 1. Try snip first (cheap)
    let snipped = snip_stale_tool_results(&events, 10);
    if utilization_after_snip <= threshold { return Ok("snipped"); }

    // 2. Fall back to summary compaction
    compact_session(session_service, cfg, &CompactStrategy::default()).await
}
```

---

## D-13: MCP OAuth (REQ-13)

### New Files

- `src/mcp_auth.rs`

### Flow

```
1. Discover auth server metadata from .well-known/oauth-authorization-server
2. Generate PKCE code_verifier + code_challenge
3. Open browser to authorization URL
4. Listen on localhost:callback_port using axum (already a dependency) as temporary HTTP server
5. Exchange authorization code for tokens
6. Store tokens in OS keychain (keyring crate)
7. On 401 from MCP server: check token expiry, refresh if expired, retry once
8. Proactive refresh: if token expires within 5 minutes, refresh before next request
```

### Config

```toml
[[profiles.default.mcp_servers]]
name = "github-mcp"
transport = "http"
endpoint = "https://mcp.github.com"
[profiles.default.mcp_servers.oauth]
client_id = "abc123"
callback_port = 8912
```

---

## D-14: Tool Search (REQ-14)

### New Files

- `src/tools/tool_search.rs`

### Implementation

```rust
pub struct ToolSearchRequest {
    pub query: String,
}

pub fn search_tools(query: &str, all_tools: &[Arc<dyn Tool>]) -> Vec<ToolSearchResult> {
    let terms: Vec<String> = query.split_whitespace()
        .map(|t| t.to_ascii_lowercase())
        .collect();
    all_tools.iter()
        .filter(|t| terms.iter().any(|term|
            t.name().to_ascii_lowercase().contains(term)
            || t.description().to_ascii_lowercase().contains(term)
        ))
        .map(|t| ToolSearchResult {
            name: t.name().to_string(),
            description: t.description().to_string(),
            parameters: t.parameters_schema(),
        })
        .collect()
}
```

### Tool Re-Registration

When tool_search returns results, the discovered tools are added to the active tool set for the remainder of the current turn. The runner maintains a `deferred_tools: Vec<Arc<dyn Tool>>` alongside the active tools. After a tool_search call, matching deferred tools are promoted to active so the LLM can call them in subsequent tool-use blocks within the same response.

---

## Dependency Changes

```toml
# New dependencies for Phase 1-3
ignore = "0.4"          # gitignore-aware walking + globset (used by glob/grep tools)
lsp-types = "0.97"      # LSP type definitions (for LSP client)
reqwest = { version = "0.12", features = ["rustls-tls"], optional = true }
htmd = "0.1"            # HTML to markdown (optional, for web_fetch)
keyring = { version = "3", optional = true }  # OS keychain for OAuth tokens

# Feature flags
[features]
web-fetch = ["dep:reqwest", "dep:htmd"]
lsp = ["dep:lsp-types"]
oauth = ["dep:keyring"]
```

---

## File Summary

| New File | REQ | Purpose |
|----------|-----|---------|
| `src/jsonrpc.rs` | 1, 2, 7 | Shared Content-Length framed JSON-RPC transport (used by MCP server, MCP stdio client, LSP client) |
| `src/mcp_server.rs` | 1 | MCP server over stdio |
| `src/tools/file_edit.rs` | 3 | String-replace edit tool |
| `src/tools/glob.rs` | 4 | Glob file search tool |
| `src/tools/grep.rs` | 5 | Ripgrep wrapper tool |
| `src/tools/bash_security.rs` | 6 | Bash command security validation |
| `src/lsp/mod.rs` | 7 | LSP module root |
| `src/lsp/manager.rs` | 7 | LSP server lifecycle |
| `src/lsp/client.rs` | 7 | LSP JSON-RPC client |
| `src/tools/lsp.rs` | 7 | LSP tool implementation |
| `src/tools/web_fetch.rs` | 8 | Web fetch tool |
| `src/tools/tool_search.rs` | 14 | Tool search tool |
| `src/mcp_auth.rs` | 13 | MCP OAuth flow |

| Modified File | REQ | Changes |
|---------------|-----|---------|
| `src/config.rs` | 2, 9, 13 | McpServerConfig transport enum, permission rules, OAuth config |
| `src/mcp.rs` | 2 | Stdio client transport |
| `src/tools/mod.rs` | 3-5, 7-8, 14 | Register new tools |
| `src/tools/execute_bash.rs` | 6 | Replace denied patterns with security pipeline |
| `src/tools/confirming.rs` | 9 | Refactor into layered pipeline |
| `src/tool_policy.rs` | 9 | Add glob pattern matching |
| `src/runner.rs` | 10, 11 | Parallel execution, fork sub-agents |
| `src/compact.rs` | 12 | Add snip strategy, auto mode |
| `src/cli.rs` | 1, 7 | Add `mcp serve`, `lsp init` commands |
| `src/lib.rs` | 1, 7 | Add `lsp`, `jsonrpc` modules |
