# zavora-cli

A Rust command-line AI agent built on [ADK-Rust](https://github.com/zavora-ai/adk-rust).

## Install

```bash
cargo install --path .
```

Requires Rust toolchain (`rustup`, `cargo`).

## Set Up a Provider

Export at least one API key:

```bash
export OPENAI_API_KEY="sk-..."
# or: GOOGLE_API_KEY, ANTHROPIC_API_KEY, DEEPSEEK_API_KEY, GROQ_API_KEY
```

Or use Ollama locally with no key needed:

```bash
OLLAMA_HOST=http://localhost:11434  # optional, this is the default
```

## Usage

```bash
# Ask a one-shot question
zavora-cli ask "Explain Rust ownership"

# Interactive chat
zavora-cli --provider openai --model gpt-4o-mini chat

# Workflows
zavora-cli workflow sequential "Plan an MVP rollout"
zavora-cli workflow graph "Draft a release plan with risks"

# Health check
zavora-cli doctor
```

## Chat Commands

Once in chat mode, these slash commands are available:

| Command | Description |
|---------|-------------|
| `/help` | Show available commands |
| `/status` | Current provider, model, session info |
| `/usage` | Context window usage breakdown by author |
| `/compact` | Compact session history to reclaim context |
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
| `/exit` | Exit chat |

---

## Built-in Tools

The agent has access to these tools during execution:

| Tool | Purpose |
|------|---------|
| `fs_read` | Read files/directories with bounded controls and workspace path policy |
| `fs_write` | Create, overwrite, append, or patch files (confirmation required) |
| `execute_bash` | Run shell commands with safety policy (read-only auto-allowed) |
| `github_ops` | GitHub operations via `gh` CLI (issues, PRs, projects) |
| `todo_list` | Create/complete/view/list/delete task lists (persisted to `.zavora/todos/`) |

## Context Management

- `/usage` shows real-time token breakdown by author (user, assistant, tool, system)
- Prompt shows ⚠ (>75%) or 🔴 (>90%) when approaching context limits
- `/compact` manually summarizes history to reclaim space
- Auto-compaction triggers when configured thresholds are exceeded

## Checkpoints and Tangents

- `/checkpoint save <label>` snapshots session state — persisted to `.zavora/checkpoints.json` across restarts
- `/tangent start` branches the session for exploratory work; `/tangent end` merges or discards

## Delegation

`/delegate <task>` runs a prompt in an isolated sub-agent session and returns the result inline.

---

## Configuration

### Profiles

Runtime defaults live in `.zavora/config.toml`:

```toml
[profiles.default]
provider = "openai"
model = "gpt-4o-mini"
session_backend = "sqlite"
session_db_url = "sqlite://.zavora/sessions.db"
retrieval_backend = "disabled"
tool_confirmation_mode = "mcp-only"
compaction_threshold = 0.75
compaction_target = 0.50
telemetry_enabled = true
guardrail_input_mode = "disabled"
guardrail_output_mode = "disabled"
guardrail_terms = ["password", "secret", "api key"]
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
model = "gpt-4o-mini"
tool_confirmation_mode = "always"
allow_tools = ["fs_read", "fs_write", "execute_bash"]
deny_tools = ["execute_bash.rm_*"]
```

```bash
zavora-cli agents list
zavora-cli agents select --name coder
zavora-cli --agent reviewer ask "Review this patch"
```

### Persistent Sessions

```bash
zavora-cli --session-backend sqlite --session-db-url sqlite://.zavora/sessions.db chat
zavora-cli --session-backend sqlite --session-db-url sqlite://.zavora/sessions.db sessions list
```

---

## Advanced Features

### Tool Policy and Hooks

Control tool execution with confirmation modes, aliases, allow/deny lists, and hooks:

```toml
[profiles.default]
tool_confirmation_mode = "mcp-only"  # never | mcp-only | always
tool_aliases = { "read_file" = "fs_read", "write_file" = "fs_write" }
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

Discover and invoke external tool servers per profile:

```toml
[[profiles.ops.mcp_servers]]
name = "ops-tools"
endpoint = "https://mcp.example.com/ops"
enabled = true
timeout_secs = 15
auth_bearer_env = "OPS_MCP_TOKEN"
tool_allowlist = ["search_incidents", "get_runbook"]
```

```bash
zavora-cli --profile ops mcp discover
```

### Guardrails

Independent input/output content policy (`disabled` | `observe` | `block` | `redact`):

```bash
zavora-cli --guardrail-input-mode block --guardrail-output-mode redact ask "Summarize this"
```

### Retrieval

Pluggable context injection from local documents or semantic search:

```bash
zavora-cli --retrieval-backend local --retrieval-doc-path ./docs/knowledge.md ask "What are our standards?"
```

### Telemetry

Structured JSONL telemetry enabled by default:

```bash
zavora-cli telemetry report --limit 2000
```

### Evaluation Harness

```bash
zavora-cli eval run --dataset evals/datasets/retrieval-baseline.v1.json --fail-under 0.90
```

### Server Mode and A2A

```bash
zavora-cli server serve --host 127.0.0.1 --port 8787
zavora-cli server a2a-smoke
```

Endpoints: `GET /healthz`, `POST /v1/ask`, `POST /v1/a2a/ping`.

---

## Development

```bash
make fmt          # format
make check        # cargo check
make lint         # clippy
make test         # unit tests
make eval         # evaluation harness
make quality-gate # eval + guardrail regression
make ci           # full CI pipeline
```

## Sensitive Config

- `profiles show`, `doctor`, and `migrate` redact session DB URLs by default
- Use `--show-sensitive-config` for local debugging when full values are needed

## Documentation

| Document | Description |
|----------|-------------|
| `CHANGELOG.md` | Release history |
| `WORKING_STYLE.md` | Sprint conventions and definition of done |
| `docs/PARITY_MATRIX.md` | Feature parity status matrix |
| `docs/PARITY_BENCHMARK.md` | Parity benchmark scenarios and scoring |
| `docs/GA_SIGNOFF_v110.md` | v1.1.0 RC sign-off |
| `docs/PHASE2_QCLI_PARITY_PLAN.md` | Phase 2 parity architecture |
| `docs/AGILE_RELEASE_CYCLE.md` | Release process |
| `docs/ADK_CAPABILITY_MATRIX.md` | ADK capability coverage |
| `docs/RETRIEVAL_ABSTRACTION.md` | Retrieval interface details |
| `docs/MCP_TOOLSET_MANAGER.md` | MCP schema and discovery flow |
| `docs/GUARDRAIL_POLICY.md` | Guardrail modes and enforcement |
| `docs/SERVER_MODE.md` | Server API and A2A flow |
| `docs/SECURITY_HARDENING.md` | Security controls |
| `docs/DIFFERENTIATION_ROADMAP.md` | Current and planned differentiators |

Current version: **v1.1.1**
