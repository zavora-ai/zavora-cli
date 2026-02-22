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
Maintainer distribution workflow: `docs/DISTRIBUTION.md`.

## Quick Start

1. Export an API key for any supported provider:

```bash
export OPENAI_API_KEY="sk-..."
# or: GOOGLE_API_KEY, ANTHROPIC_API_KEY, DEEPSEEK_API_KEY, GROQ_API_KEY
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
# Ask a one-shot question
zavora-cli ask "Explain Rust ownership"

# Interactive chat with a specific model
zavora-cli --provider openai --model gpt-4.1 chat

# Workflows
zavora-cli workflow sequential "Plan an MVP rollout"
zavora-cli workflow graph "Draft a release plan with risks"

# Health check
zavora-cli doctor
```

## Multi-Agent Orchestration

The assistant uses a **capability + workflow** agent architecture by default:

**Capability Agents** (callable as tools):
- **time_agent** — Current time context, parse relative dates ("next Friday", "in 2 days")
- **memory_agent** — Persistent learnings across sessions (stored in `.zavora/memory.json`)
- **search_agent** — Web search via Gemini's Google Search (requires Gemini model)

**Workflow Agents** (execution patterns):
- **file_search_agent** — Iterative file discovery with saturation detection
- **sequential_agent** — Plan creation and step-by-step execution with progress tracking
- **quality_agent** — Verification against acceptance criteria

**Orchestration Pattern:**
```
Bootstrap (time + memory) → Gather (search/files) → Plan → Execute → Verify → Commit
```

The orchestrator automatically:
- Recalls relevant memories at the start of tasks
- Uses time context for time-sensitive work
- Delegates to workflow agents for complex multi-step tasks
- Stores high-signal learnings after successful completions

The LLM can call capability agents as tools, or you can use chat commands:
- `/memory recall|remember|forget` — Manage persistent learnings
- `/time [query]` — Get time context or parse dates
- `/orchestrate <goal>` — Run full orchestration loop

## Chat Commands

| Command | Description |
|---------|-------------|
| `/help` | Show available commands |
| `/status` | Current provider, model, session info |
| `/usage` | Context window usage breakdown by author |
| `/compact` | Compact session history to reclaim context |
| `/autocompact` | Toggle automatic compaction (threshold-based) |
| `/memory <cmd>` | recall\|remember\|forget persistent learnings |
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
| `/delegate <task>` | Run isolated sub-agent prompt |
| `/provider <name>` | Switch provider mid-session |
| `/model [id]` | Switch model or open picker |
| `/agent` | Trust all tools for the session (agent mode) |
| `/exit` | Exit chat |

## Built-in Tools

| Tool | Purpose |
|------|---------|
| `fs_read` | Read files and directories with workspace path policy |
| `fs_write` | Create, overwrite, append, or patch files (confirmation required) |
| `execute_bash` | Run shell commands with safety policy (read-only commands auto-approved) |
| `github_ops` | GitHub operations via `gh` CLI (issues, PRs, projects) |
| `todo_list` | Create/complete/view/list/delete task lists (persisted to `.zavora/todos/`) |
| `time_agent` | Current time context and relative date parsing |
| `memory_agent` | Persistent learnings storage and recall |

## Context Management

- `/usage` shows real-time token breakdown by author (user, assistant, tool, system)
- Prompt shows ⚠ (>80%) or 🔴 (>90%) when approaching context limits
- `/compact` manually summarizes history to reclaim space
- `/autocompact` toggles automatic compaction (default: enabled at 75% → 10%)
- Auto-compaction uses LLM-generated structured summaries

## Configuration

Runtime defaults live in `.zavora/config.toml`:

```toml
[profiles.default]
provider = "openai"
model = "gpt-4.1"
session_backend = "sqlite"
session_db_url = "sqlite://.zavora/sessions.db"
retrieval_backend = "disabled"
tool_confirmation_mode = "mcp-only"
auto_compact_enabled = true
compaction_threshold = 0.75
compaction_target = 0.10
telemetry_enabled = true
```

```bash
zavora-cli profiles list
zavora-cli --profile ops profiles show
```

### Agent Catalogs

Configure agent personas separately from profiles. Precedence: implicit `default` → global `~/.zavora/agents.toml` → local `.zavora/agents.toml`.

```toml
[agents.coder]
description = "Code-focused assistant"
provider = "openai"
model = "gpt-4.1"
tool_confirmation_mode = "always"
allow_tools = ["fs_read", "fs_write", "execute_bash"]
```

```bash
zavora-cli agents list
zavora-cli --agent reviewer ask "Review this patch"
```

## Advanced Features

### Tool Policy and Hooks

```toml
[profiles.default]
tool_confirmation_mode = "mcp-only"  # never | mcp-only | always
tool_timeout_secs = 45
tool_retry_attempts = 2

[[profiles.default.hooks.pre_tool]]
name = "block-rm"
match_tool = "execute_bash"
match_args = "rm -rf"
action = "block"
message = "Destructive rm blocked by hook policy"
```

### MCP Integration

```toml
[[profiles.ops.mcp_servers]]
name = "ops-tools"
endpoint = "https://mcp.example.com/ops"
enabled = true
timeout_secs = 15
auth_bearer_env = "OPS_MCP_TOKEN"
tool_allowlist = ["search_incidents", "get_runbook"]
```

### Guardrails

Independent input/output content policy (`disabled` | `observe` | `block` | `redact`):

```bash
zavora-cli --guardrail-input-mode block --guardrail-output-mode redact ask "Summarize this"
```

### Retrieval

```bash
zavora-cli --retrieval-backend local --retrieval-doc-path ./docs/knowledge.md ask "What are our standards?"
```

### Server Mode and A2A

```bash
zavora-cli server serve --host 127.0.0.1 --port 8787
```

Endpoints: `GET /healthz`, `POST /v1/ask`, `POST /v1/a2a/ping`.

## Development

```bash
make fmt          # format
make check        # cargo check
make lint         # clippy
make test         # unit tests
make ci           # full CI pipeline
```

## License

MIT
