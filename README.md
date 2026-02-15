# zavora-cli

A Rust command-line AI agent built on [ADK-Rust](https://github.com/zavora-ai/adk-rust).
Multi-provider, tool-augmented, with persistent sessions, context management, and coding workflows.

## Prerequisites

- Rust toolchain (`rustup`, `cargo`)
- At least one provider credential (or Ollama running locally)

Set one or more of:

```
GOOGLE_API_KEY
OPENAI_API_KEY
ANTHROPIC_API_KEY
DEEPSEEK_API_KEY
GROQ_API_KEY
OLLAMA_HOST          # optional, defaults to http://localhost:11434
```

## Quick Start

```bash
cargo run -- ask "Design a Rust CLI with release-based milestones"
cargo run -- --provider openai --model gpt-4o-mini chat
cargo run -- workflow sequential "Plan an MVP rollout with quality gates"
cargo run -- workflow graph "Draft a release rollout plan with risks"
cargo run -- release-plan "Build an enterprise-ready AI CLI" --releases 3
cargo run -- doctor
```

## Core Capabilities

- **Providers**: `gemini`, `openai`, `anthropic`, `deepseek`, `groq`, `ollama`
- **Session backends**: `memory` (in-process) and `sqlite` (persistent across restarts)
- **ADK workflow modes**: `single`, `sequential`, `parallel`, `loop`, `graph`
- **Built-in tools**: `fs_read`, `fs_write`, `execute_bash`, `github_ops`, `todo_list`
- **MCP integration**: discover and invoke external tool servers per profile
- **Retrieval**: pluggable context injection (`disabled`, `local`, `semantic`)

## Chat Mode

Interactive chat with slash commands, context-aware prompts, and streaming output.

```bash
cargo run -- --provider openai --model gpt-4o-mini chat
```

### Slash Commands

| Command | Description |
|---------|-------------|
| `/help` | Show available commands |
| `/status` | Current provider, model, session info |
| `/tools` | List active built-in and MCP tools |
| `/mcp` | MCP server status and diagnostics |
| `/usage` | Real-time context window usage breakdown by author |
| `/compact` | Manually compact session history to reclaim context |
| `/checkpoint save <label>` | Save session state snapshot |
| `/checkpoint list` | List saved checkpoints |
| `/checkpoint restore <tag>` | Restore session to a checkpoint |
| `/tangent start` | Branch into an exploratory tangent |
| `/tangent end` | Return to main session, optionally keeping tangent work |
| `/todos list` | List todo lists in workspace |
| `/todos show <id>` | Show a specific todo list |
| `/todos clear` | Remove finished todo lists |
| `/delegate <task>` | Run an isolated sub-agent prompt in a separate session |
| `/provider <name>` | Switch provider mid-session |
| `/model [id]` | Switch model or open interactive model picker |
| `/exit` | Exit chat |

### Context Management

- **Context usage**: computed from real session events — `/usage` shows token breakdown by author (user, assistant, tool, system) with provider-aware context window limits
- **Budget indicators**: prompt shows ⚠ (>75%) or 🔴 (>90%) when approaching context limits
- **Auto-compaction**: configurable automatic summarization when context exceeds thresholds
- **Manual compaction**: `/compact` summarizes history on demand

### Checkpoints and Tangents

- **Checkpoints** persist to `.zavora/checkpoints.json` and survive CLI restarts
- **Tangent mode** branches the session for exploratory work, then merges or discards

### Todos and Delegation

- **Todo lists** are persisted to `.zavora/todos/` as JSON files
- The `todo_list` built-in tool lets the agent create/complete/view/list/delete todos during execution
- `/delegate <task>` runs an isolated sub-agent prompt in a separate session and returns the result

## Built-in Tools

### `fs_read`

File and directory inspection with bounded controls.

- `start_line`, `max_lines`, `max_bytes` for files; `max_entries` for directories
- Workspace path policy: denies paths outside root and blocked segments (`.git`, `.zavora`, `.env*`)

### `fs_write`

Controlled file mutations.

- Modes: `create`, `overwrite`, `append`, `patch`
- `patch` applies minimal-diff replacement via `{ find, replace, replace_all }`
- Requires tool confirmation by default

### `execute_bash`

Shell execution with safety policy.

- Read-only commands auto-allowed (`ls`, `cat`, `rg`, `git status`, `git diff`)
- Dangerous patterns blocked by default
- Per-call `timeout_secs`, `retry_attempts`, `retry_delay_ms`, `max_output_chars`

### `github_ops`

GitHub workflow operations via `gh` CLI.

- Actions: `issue_create`, `issue_update`, `pr_create`, `project_item_update`
- Requires `GH_TOKEN`/`GITHUB_TOKEN` or `gh auth status`

### `todo_list`

Task list management for structured agent execution.

- Actions: `create`, `complete`, `view`, `list`, `delete`
- Persisted to `.zavora/todos/<id>.json`

## Tool Policy and Hooks

### Tool Confirmation

```toml
[profiles.default]
tool_confirmation_mode = "mcp-only"  # never | mcp-only | always
require_confirm_tool = ["release_template"]
approve_tool = ["search_incidents"]
```

### Tool Aliases

Remap tool names for agent compatibility:

```toml
[profiles.default]
tool_aliases = { "read_file" = "fs_read", "write_file" = "fs_write" }
```

### Tool Allow/Deny Lists

Per-agent tool filtering with wildcard support:

```toml
[agents.coder]
allow_tools = ["fs_read", "fs_write", "execute_bash", "github_ops.*"]
deny_tools = ["execute_bash.rm_*"]
```

### Hooks

Pre/post-tool hooks for validation, logging, or blocking:

```toml
[[profiles.default.hooks.pre_tool]]
name = "audit-log"
command = "echo 'tool invoked: $TOOL_NAME' >> .zavora/audit.log"
timeout_secs = 5

[[profiles.default.hooks.pre_tool]]
name = "block-rm"
match_tool = "execute_bash"
match_args = "rm -rf"
action = "block"
message = "Destructive rm blocked by hook policy"
```

### Tool Reliability

```toml
[profiles.default]
tool_timeout_secs = 45
tool_retry_attempts = 2
tool_retry_delay_ms = 500
```

## Profiles

Runtime defaults from `.zavora/config.toml` (override with `--config-path`):

```toml
[profiles.default]
provider = "openai"
model = "gpt-4o-mini"
session_backend = "sqlite"
session_db_url = "sqlite://.zavora/sessions.db"
app_name = "zavora-cli"
user_id = "local-user"
session_id = "default-session"
retrieval_backend = "disabled"
tool_confirmation_mode = "mcp-only"
telemetry_enabled = true
telemetry_path = ".zavora/telemetry/events.jsonl"
guardrail_input_mode = "disabled"
guardrail_output_mode = "disabled"
guardrail_terms = ["password", "secret", "api key"]
guardrail_redact_replacement = "[REDACTED]"
compaction_threshold = 0.75
compaction_target = 0.50
```

```bash
cargo run -- profiles list
cargo run -- --profile ops profiles show
```

## Agent Catalogs

Configure agent behavior separately from profiles.

Precedence: implicit `default` → global `~/.zavora/agents.toml` → local `.zavora/agents.toml`.

```toml
[agents.coder]
description = "Code-focused assistant"
provider = "openai"
model = "gpt-4o-mini"
tool_confirmation_mode = "always"
resource_paths = ["docs/architecture.md"]
allow_tools = ["fs_read", "fs_write", "execute_bash"]
deny_tools = ["execute_bash.rm_*"]
```

```bash
cargo run -- agents list
cargo run -- agents show --name coder
cargo run -- agents select --name coder
cargo run -- --agent reviewer ask "Review this patch"
```

## MCP Toolset Manager

```bash
cargo run -- --profile ops mcp list
cargo run -- --profile ops mcp discover
cargo run -- --profile ops mcp discover --server ops-tools
```

```toml
[[profiles.ops.mcp_servers]]
name = "ops-tools"
endpoint = "https://mcp.example.com/ops"
enabled = true
timeout_secs = 15
auth_bearer_env = "OPS_MCP_TOKEN"
tool_allowlist = ["search_incidents", "get_runbook"]
```

- Unreachable servers show categorized diagnostics in `mcp discover`
- Unavailable servers are skipped at runtime with a warning

## Guardrails

Independent input/output content policy:

- `disabled` | `observe` | `block` | `redact`

```toml
[profiles.default]
guardrail_input_mode = "observe"
guardrail_output_mode = "redact"
guardrail_terms = ["password", "secret", "api key", "private key"]
guardrail_redact_replacement = "[REDACTED]"
```

```bash
cargo run -- --guardrail-input-mode block --guardrail-output-mode redact ask "Summarize this"
```

## Persistent Sessions

```bash
cargo run -- \
  --session-backend sqlite \
  --session-db-url sqlite://.zavora/sessions.db \
  --session-id team-planning \
  chat
```

```bash
cargo run -- --session-backend sqlite --session-db-url sqlite://.zavora/sessions.db sessions list
cargo run -- --session-backend sqlite --session-db-url sqlite://.zavora/sessions.db sessions show --recent 30
cargo run -- --session-backend sqlite --session-db-url sqlite://.zavora/sessions.db sessions delete --session-id old --force
cargo run -- --session-backend sqlite --session-db-url sqlite://.zavora/sessions.db sessions prune --keep 20 --dry-run
```

## Retrieval

```bash
cargo run -- --retrieval-backend local --retrieval-doc-path ./docs/knowledge.md ask "What are our standards?"
cargo run --features semantic-search -- --retrieval-backend semantic --retrieval-doc-path ./docs/knowledge.md ask "Rollout guardrails?"
```

## Telemetry

Structured JSONL telemetry enabled by default.

```bash
cargo run -- telemetry report --limit 2000
```

Summarizes: event counts, command runs, tool lifecycle (`requested`, `succeeded`, `failed`).

## Evaluation Harness

```bash
cargo run -- eval run --dataset evals/datasets/retrieval-baseline.v1.json --fail-under 0.90
```

Produces: per-case pass/fail, aggregate pass rate, benchmark metrics (`avg_latency_ms`, `p95_latency_ms`, `throughput_qps`).

## Server Mode and A2A

```bash
cargo run -- server serve --host 127.0.0.1 --port 8787
cargo run -- server a2a-smoke
```

Endpoints: `GET /healthz`, `POST /v1/ask`, `POST /v1/a2a/ping`.

## Development

```bash
make fmt          # format
make check        # cargo check
make lint         # clippy
make test         # unit tests
make eval         # evaluation harness
make quality-gate # eval + guardrail regression
make security-check
make perf-check
make ci           # full CI pipeline
make release-check
```

## Sensitive Config

- `profiles show`, `doctor`, and `migrate` redact session DB URLs by default
- Use `--show-sensitive-config` for local debugging when full values are needed

## Release Model

SemVer release train: plan by release slices, merge behind CI, tag stable increments.

Current version: **v1.1.1**

## Documentation

| Document | Description |
|----------|-------------|
| `CHANGELOG.md` | Release history |
| `WORKING_STYLE.md` | Sprint conventions and definition of done |
| `docs/PHASE2_QCLI_PARITY_PLAN.md` | Phase 2 parity architecture and sprint plan |
| `docs/PARITY_MATRIX.md` | Feature parity status matrix |
| `docs/PARITY_BENCHMARK.md` | Parity benchmark scenarios and scoring |
| `docs/GA_SIGNOFF.md` | GA completion checklist |
| `docs/GA_SIGNOFF_v110.md` | v1.1.0 RC sign-off |
| `docs/AGILE_RELEASE_CYCLE.md` | Release process |
| `docs/PROJECT_PLAN.md` | Sprint roadmap |
| `docs/ADK_CAPABILITY_MATRIX.md` | ADK capability coverage |
| `docs/ADK_TARGET_ARCHITECTURE.md` | Target architecture |
| `docs/RETRIEVAL_ABSTRACTION.md` | Retrieval interface details |
| `docs/MCP_TOOLSET_MANAGER.md` | MCP schema and discovery flow |
| `docs/GRAPH_WORKFLOWS.md` | Graph routing and templates |
| `docs/GUARDRAIL_POLICY.md` | Guardrail modes and enforcement |
| `docs/QUALITY_GATES.md` | CI/release gate thresholds |
| `docs/SERVER_MODE.md` | Server API and A2A flow |
| `docs/SECURITY_HARDENING.md` | Security controls |
| `docs/PERFORMANCE_RELIABILITY.md` | Load targets and perf summary |
| `docs/OPERATOR_RUNBOOK.md` | Operational procedures |
| `docs/MIGRATION_GUIDE_v1.md` | Pre-1.0 to v1.0.0 upgrade |
| `docs/EVAL_BASELINE.md` | Eval dataset baseline metrics |
| `docs/DIFFERENTIATION_ROADMAP.md` | Current and planned differentiators |

Temporary upstream RustSec exceptions are tracked in `.cargo/audit.toml` and reviewed each release.
