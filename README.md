# zavora-cli

[![Crates.io](https://img.shields.io/crates/v/zavora-cli.svg)](https://crates.io/crates/zavora-cli)
[![License: MIT](https://img.shields.io/badge/license-MIT-blue.svg)](LICENSE)

```
  ███████╗ █████╗ ██╗   ██╗ ██████╗ ██████╗  █████╗
  ╚══███╔╝██╔══██╗██║   ██║██╔═══██╗██╔══██╗██╔══██╗
    ███╔╝ ███████║██║   ██║██║   ██║██████╔╝███████║
   ███╔╝  ██╔══██║╚██╗ ██╔╝██║   ██║██╔══██╗██╔══██║
  ███████╗██║  ██║ ╚████╔╝ ╚██████╔╝██║  ██║██║  ██║
  ╚══════╝╚═╝  ╚═╝  ╚═══╝  ╚═════╝ ╚═╝  ╚═╝╚═╝  ╚═╝
```

**Your AI agent, in the terminal.** Built on [ADK-Rust](https://github.com/zavora-ai/adk-rust).

Multi-agent orchestration, tool safety controls, streaming markdown, checkpoints, MCP integration — all from a single binary.

## Install

### Cargo (recommended)

```bash
cargo install zavora-cli
```

### npm

```bash
npm i -g @zavora-ai/zavora-cli
```

### Homebrew

```bash
brew install --formula https://raw.githubusercontent.com/zavora-ai/zavora-cli/main/Formula/zavora-cli.rb
```

### Build from source

```bash
git clone https://github.com/zavora-ai/zavora-cli.git
cd zavora-cli
cargo install --path .
```

Requires Rust 1.85+ (`rustup`, `cargo`) for cargo/source builds.

## Quick Start

1. Export an API key for any supported provider:

```bash
export GOOGLE_API_KEY="..."
# or: OPENAI_API_KEY, ANTHROPIC_API_KEY, DEEPSEEK_API_KEY, GROQ_API_KEY
```

Or use Ollama locally with no key needed:

```bash
OLLAMA_HOST=http://localhost:11434  # optional, this is the default
```

2. Start chatting:

```bash
zavora-cli chat
```

## Usage

```bash
# Interactive chat (default when no subcommand)
zavora-cli chat
zavora-cli                          # same as above

# One-shot question
zavora-cli ask "Explain Rust ownership"

# Specific provider/model
zavora-cli --provider gemini --model gemini-2.5-flash chat

# Workflows
zavora-cli workflow sequential "Plan an MVP rollout"
zavora-cli workflow graph "Draft a release plan with risks"

# Skills
zavora-cli skills list              # list discovered skills

# RAG (requires --features rag)
zavora-cli rag ingest ./docs/       # ingest documents

# Ralph autonomous dev pipeline
zavora-cli ralph "Build a REST API for user management"

# Management
zavora-cli profiles list
zavora-cli agents list
zavora-cli sessions list
zavora-cli mcp list
zavora-cli doctor
```

## Architecture

```
main.rs
  ├── init_tracing()              console + optional OTLP layer (composable)
  ├── memory::init()              single SQLite pool, OnceLock singleton
  ├── resolve_runtime_tools()     builtin + MCP + browser (feature-gated)
  └── build_runner()
        ├── .memory_service()     ← shared memory singleton
        ├── .with_auto_skills_mut()  .skills/ + .claude/skills/
        └── .compaction_config()

Memory (single source of truth):
  main.rs::init() → OnceLock<Arc<MemoryServiceAdapter>>
       ↓                          ↓
  Runner (.memory_service)    /memory commands + memory_agent tool

Two retrieval layers:
  retrieval.rs    auto prompt enrichment (before LLM call)
  tools/rag.rs    LLM-callable on-demand retrieval (feature-gated)
```

## Multi-Agent Orchestration

**Capability Agents** (callable as tools):
- **time_agent** — Current time context, parse relative dates ("next Friday", "in 2 days")
- **memory_agent** — Persistent learnings across sessions (SQLite-backed, FTS5 search)
- **search_agent** — Web search via Gemini's Google Search (requires Gemini provider)

**Workflow Agents** (execution patterns):
- **file_search_agent** — Iterative file discovery with saturation detection
- **sequential_agent** — Plan creation and step-by-step execution with progress tracking
- **quality_agent** — Verification against acceptance criteria

**Orchestration Pattern:**
```
Bootstrap (time + memory) → Gather (search/files) → Plan → Execute → Verify → Commit
```

## ADK Crate Integrations

| Crate | Purpose | Integration |
|-------|---------|-------------|
| `adk-skill` | Skill system | Auto-discovers `.skills/`, `.claude/skills/`, `~/.zavora/skills/` |
| `adk-memory` | Semantic memory | SQLite FTS5 via `SqliteMemoryService`; shared singleton for Runner + chat |
| `adk-telemetry` | Observability | Composable OTLP layer via `OTEL_EXPORTER_OTLP_ENDPOINT`; console fallback |
| `adk-guardrail` | Safety | `PiiRedactor` (email/phone/SSN/CC) + `ContentFilter` (blocked keywords) |
| `adk-browser` | Browser automation | 40+ WebDriver tools (feature: `browser`) |
| `adk-sandbox` | Code execution | Sandboxed Python/Node/Rust via ProcessBackend (feature: `sandbox`) |
| `adk-rag` | RAG pipeline | InMemoryVectorStore + bag-of-words embedding (feature: `rag`) |

### Skills

Place `.md` files with YAML frontmatter in `.skills/` or `.claude/skills/`:

```yaml
---
name: my-skill
description: When to use this skill
---
# Instructions here
```

Compatible with [Anthropic's skills](https://github.com/anthropics/skills) format.

### File History and /undo

Every `fs_write` (overwrite/append) and `file_edit` automatically snapshots the file before modification. Snapshots are stored in `.zavora/file_history/` (max 20 per file, oldest pruned). Use `/undo` in chat to restore the last modified file.

## Feature Flags

| Feature | What it enables |
|---------|----------------|
| `browser` | 40+ browser automation tools via WebDriver (`adk-browser`) |
| `sandbox` | Sandboxed code execution: Python, Node.js, Rust (`adk-sandbox`) |
| `rag` | RAG pipeline with `zavora rag ingest <path>` (`adk-rag`) |
| `web-fetch` | HTTP fetch with HTML→markdown conversion |
| `lsp` | Language Server Protocol: definitions, references, hover, symbols |
| `oauth` | MCP OAuth 2.0 PKCE flow with OS keychain storage |

```bash
# Build with all optional features
cargo install zavora-cli --features "web-fetch,lsp,oauth,browser,sandbox,rag"
```

## Chat Commands

| Command | Description |
|---------|-------------|
| `/help` | Show available commands |
| `/status` | Current provider, model, session info |
| `/usage` | Context window usage breakdown by author |
| `/compact` | Compact session history to reclaim context |
| `/autocompact` | Toggle automatic compaction (threshold-based) |
| `/memory recall [query]` | Search memories (empty = list all) |
| `/memory remember <text>` | Store a persistent memory |
| `/memory forget <query>` | Delete matching memories |
| `/time [query]` | Get time context or parse relative dates |
| `/orchestrate <goal>` | Run full agent orchestration loop |
| `/tools` | List active built-in and MCP tools |
| `/mcp` | MCP server diagnostics |
| `/checkpoint save <label>` | Save session snapshot |
| `/checkpoint list` | List saved checkpoints |
| `/checkpoint restore <tag>` | Restore to a checkpoint |
| `/tangent start` | Branch into exploratory tangent |
| `/tangent end` | Return to main session |
| `/todos list` | List todo lists |
| `/todos show <id>` | Show a todo list |
| `/todos clear` | Remove finished todos |
| `/delegate <task>` | Fork isolated sub-agent (fresh context, 5-min timeout) |
| `/allow <pattern>` | Auto-approve tool pattern for this session |
| `/deny <pattern>` | Deny tool pattern for this session |
| `/undo` | Restore last modified file from snapshot |
| `/ralph <prompt>` | Run Ralph autonomous dev pipeline |
| `/provider <name>` | Switch provider mid-session |
| `/model [id]` | Switch model or open picker |
| `/agent` | Trust all tools for the session (agent mode) |
| `/exit` | Exit chat |

## Built-in Tools

| Tool | Purpose | Read-only |
|------|---------|-----------|
| `current_unix_time` | Current UTC timestamp | ✅ |
| `fs_read` | Read files and directories with workspace path policy | ✅ |
| `fs_write` | Create, overwrite, append, or patch files | ❌ |
| `file_edit` | Surgical `old_string → new_string` replacement with diff output | ❌ |
| `execute_bash` | Run shell commands with 20-check security pipeline | ❌ |
| `glob` | Find files by glob pattern, respects `.gitignore` | ✅ |
| `grep` | Search file contents via ripgrep with context lines | ✅ |
| `github_ops` | GitHub operations via `gh` CLI | ❌ |
| `todo_list` | Create/complete/view/list/delete task lists | ❌ |
| `time_agent` | Current time context and relative date parsing | ✅ |
| `memory_agent` | Persistent learnings: recall, remember, forget | ❌ |
| `release_template` | Agile release checklist skeleton | ✅ |
| `tool_search` | Keyword discovery of available tools (auto-enabled >15 tools) | ✅ |
| `web_fetch` | Fetch URLs as markdown (feature: `web-fetch`) | ✅ |
| `lsp` | Code intelligence: 9 operations, 7 languages (feature: `lsp`) | ✅ |
| `code_execute` | Sandboxed code execution (feature: `sandbox`) | ❌ |
| `rag_search` | RAG retrieval from ingested documents (feature: `rag`) | ✅ |
| `browser_*` | 40+ browser automation tools (feature: `browser`) | ❌ |

## Context Management

- `/usage` shows real-time token breakdown by author (user, assistant, tool, system)
- Prompt shows ⚠ (>80%) or 🔴 (>90%) when approaching context limits
- `/compact` manually summarizes history to reclaim space
- `/autocompact` toggles automatic compaction (default: enabled at 75% → 10%)
- Auto-compaction uses snip-first strategy (removes stale tool results) then LLM summary fallback
- `/delegate <task>` forks an isolated sub-agent with fresh context and 5-minute timeout

## Configuration

Runtime defaults live in `.zavora/config.toml`:

```toml
[profiles.default]
provider = "gemini"
model = "gemini-2.5-flash"
session_backend = "sqlite"
session_db_url = "sqlite://.zavora/sessions.db"
retrieval_backend = "disabled"
tool_confirmation_mode = "mcp-only"
auto_compact_enabled = true
compaction_threshold = 0.75
compaction_target = 0.10
telemetry_enabled = true
```

### Telemetry

Console tracing is always active. Set `OTEL_EXPORTER_OTLP_ENDPOINT` to enable OpenTelemetry export to Jaeger, Datadog, etc. Both layers compose on the same subscriber — no conflict.

```bash
OTEL_EXPORTER_OTLP_ENDPOINT=http://localhost:4317 zavora-cli chat
```

### Guardrails

PII redaction (emails, phones, SSNs, credit cards) is automatic in redact mode. Custom blocked keywords are configurable.

```bash
zavora-cli --guardrail-input-mode block --guardrail-output-mode redact ask "Summarize this"
```

### MCP Integration

**As a client** — connect to HTTP or stdio MCP servers:

```toml
[[profiles.ops.mcp_servers]]
name = "filesystem"
command = "npx"
args = ["-y", "@modelcontextprotocol/server-filesystem", "/path"]
```

**As a server** — expose zavora's tools to any MCP client:

```bash
zavora-cli mcp serve
```

### Permission Rules

```toml
[profiles.default.permission_rules]
always_allow = ["fs_read:*", "glob:*", "grep:*", "execute_bash:git status*"]
always_deny = ["execute_bash:rm -rf *", "fs_write:/etc/*"]
always_ask = ["web_fetch:*", "github_ops:*"]
```

Session-level: `/allow execute_bash:cargo *` and `/deny fs_write:*.env`

### Server Mode

```bash
zavora-cli server serve --host 127.0.0.1 --port 8787
```

Endpoints: `GET /healthz`, `POST /v1/ask`, `POST /v1/a2a/ping`.

## Development

```bash
cargo check                         # type check
cargo test -- --test-threads=1      # 210 tests
cargo check --features "browser,sandbox,rag,lsp,web-fetch,oauth"  # all features
```

## License

MIT
