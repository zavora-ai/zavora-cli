# zavora-cli

`zavora-cli` is a Rust command-line AI agent built on [ADK-Rust](https://github.com/zavora-ai/adk-rust).
It is structured for fast iteration and release-based delivery with CI/release automation.

## What It Supports

- Provider-aware model runtime (`gemini`, `openai`, `anthropic`, `deepseek`, `groq`, `ollama`)
- Session backends (`memory` and persistent `sqlite`)
- ADK workflow modes:
  - `single` (`LlmAgent`)
  - `sequential` (`SequentialAgent`)
  - `parallel` (`ParallelAgent` + synthesis stage)
  - `loop` (`LoopAgent` + `ExitLoopTool`)
  - `graph` (`GraphAgent` with conditional routing and reusable templates)
- Session-backed execution with `Runner`
- Built-in release planning command for agile slices

## Prerequisites

- Rust toolchain (`rustup`, `cargo`)
- One provider credential (or Ollama running locally)

Set at least one of:

- `GOOGLE_API_KEY`
- `OPENAI_API_KEY`
- `ANTHROPIC_API_KEY`
- `DEEPSEEK_API_KEY`
- `GROQ_API_KEY`
- For local: `OLLAMA_HOST` (optional, defaults to `http://localhost:11434`)

## Quick Start

```bash
cargo run -- ask "Design a Rust CLI with release-based milestones"
```

```bash
cargo run -- --provider openai --model gpt-4o-mini chat
```

```bash
cargo run -- workflow sequential "Plan an MVP rollout with quality gates"
```

```bash
cargo run -- workflow graph "Draft a release rollout plan with risks"
```

```bash
cargo run -- release-plan "Build an enterprise-ready AI CLI" --releases 3
```

```bash
cargo run -- doctor
```

```bash
cargo run -- eval run --benchmark-iterations 200 --fail-under 0.90
```

```bash
cargo run -- --session-backend sqlite --session-db-url sqlite://.zavora/sessions.db migrate
```

```bash
cargo run -- --session-backend sqlite --session-db-url sqlite://.zavora/sessions.db sessions list
```

```bash
cargo run -- --session-backend sqlite --session-db-url sqlite://.zavora/sessions.db --session-id team-planning sessions show --recent 30
```

```bash
cargo run -- --session-backend sqlite --session-db-url sqlite://.zavora/sessions.db sessions delete --session-id team-planning --force
```

```bash
cargo run -- --session-backend sqlite --session-db-url sqlite://.zavora/sessions.db sessions prune --keep 20 --dry-run
```

Session retention behavior:
- `memory` backend: in-process only; data resets when the CLI process exits.
- `sqlite` backend: persistent across runs; `sessions delete` and `sessions prune` are destructive and require `--force` (or `--dry-run` for preview).

## Development Commands

Use `make` targets:

- `make fmt`
- `make fmt-check`
- `make check`
- `make lint`
- `make test`
- `make eval`
- `make ci`
- `make release-check`

## Profiles

`zavora-cli` can resolve runtime defaults from profiles in `.zavora/config.toml` (override path with `--config-path`).

Example profile config:

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
retrieval_max_chunks = 3
retrieval_max_chars = 4000
retrieval_min_score = 1
tool_confirmation_mode = "mcp-only"
require_confirm_tool = []
approve_tool = []
tool_timeout_secs = 45
tool_retry_attempts = 2
tool_retry_delay_ms = 500
telemetry_enabled = true
telemetry_path = ".zavora/telemetry/events.jsonl"

[profiles.ops]
provider = "anthropic"
model = "claude-sonnet-4-20250514"
session_backend = "sqlite"
session_db_url = "sqlite://.zavora/ops-sessions.db"
retrieval_backend = "local"
retrieval_doc_path = "docs/ops-knowledge.md"
retrieval_max_chunks = 4
retrieval_max_chars = 3000
retrieval_min_score = 2

[[profiles.ops.mcp_servers]]
name = "ops-tools"
endpoint = "https://mcp.example.com/ops"
enabled = true
timeout_secs = 15
auth_bearer_env = "OPS_MCP_TOKEN"
tool_allowlist = ["search_incidents", "get_runbook"]
```

Inspect profile state:

```bash
cargo run -- profiles list
cargo run -- --profile ops profiles show
```

## MCP Toolset Manager

Configure MCP servers per profile and use CLI discovery commands:

```bash
cargo run -- --profile ops mcp list
cargo run -- --profile ops mcp discover
cargo run -- --profile ops mcp discover --server ops-tools
```

Behavior:
- Enabled MCP servers are discovered with per-server timeout/auth settings.
- Unreachable servers fail with categorized tooling errors in `mcp discover`.
- Runtime command execution (`ask`, `chat`, `workflow single`) loads built-in tools plus discovered MCP tools.
- If an MCP server is unavailable during runtime tool discovery, it is skipped with a warning and execution continues.

## Tool Confirmation Safety Controls

`zavora-cli` uses ADK-Rust tool confirmation policy controls with a safe default:
- default mode is `mcp-only`: discovered MCP tools require explicit approval
- required-but-unapproved tools are denied deterministically
- policy is configurable per profile and per tool

Profile fields:

```toml
[profiles.default]
tool_confirmation_mode = "mcp-only" # never | mcp-only | always
require_confirm_tool = ["release_template"] # optional extra required tools
approve_tool = ["search_incidents"] # allow specific required tools
```

CLI overrides:

```bash
cargo run -- \
  --tool-confirmation-mode always \
  --require-confirm-tool release_template \
  --approve-tool release_template \
  ask "Create a release checklist"
```

Inspect active policy with:

```bash
cargo run -- profiles show
cargo run -- doctor
```

## Tool Reliability Controls

Tool execution reliability is configurable at profile/CLI level:

```toml
[profiles.default]
tool_timeout_secs = 45
tool_retry_attempts = 2
tool_retry_delay_ms = 500
```

CLI override example:

```bash
cargo run -- \
  --tool-timeout-secs 60 \
  --tool-retry-attempts 3 \
  --tool-retry-delay-ms 800 \
  mcp discover
```

Runtime behavior:
- single-agent tool timeout is enforced via ADK `tool_timeout`
- MCP discovery/invocation retries follow configured retry attempts/delay
- tool lifecycle telemetry emits structured events for `requested`, `succeeded`, and `failed`

## Telemetry Baseline and Reporting

Structured telemetry is enabled by default and written as JSONL.

Profile/runtime controls:

```toml
[profiles.default]
telemetry_enabled = true
telemetry_path = ".zavora/telemetry/events.jsonl"
```

CLI/env overrides:

```bash
cargo run -- \
  --telemetry-enabled false \
  --telemetry-path .zavora/telemetry/custom-events.jsonl \
  doctor
```

- `ZAVORA_TELEMETRY_ENABLED=true|false`
- `ZAVORA_TELEMETRY_PATH=<path>`

Minimal dashboard report:

```bash
cargo run -- telemetry report --limit 2000
```

The report summarizes:
- parsed events and parse errors
- unique command runs
- command completion/failure counts
- tool lifecycle counts (`requested`, `succeeded`, `failed`)

## Evaluation Harness and Benchmark Suite

Run dataset-based quality evaluation and retrieval benchmark metrics:

```bash
cargo run -- \
  eval run \
  --dataset evals/datasets/retrieval-baseline.v1.json \
  --output .zavora/evals/latest.json \
  --benchmark-iterations 200 \
  --fail-under 0.90
```

What the eval command produces:
- pass/fail quality score per dataset case
- aggregate pass rate
- benchmark metrics (`avg_latency_ms`, `p95_latency_ms`, `throughput_qps`)
- machine-readable JSON report for release artifacts

Release baseline reports are tracked under `evals/reports/` and summarized in `docs/EVAL_BASELINE.md`.

## Retrieval Abstraction

Retrieval is pluggable and disabled by default.

- `disabled`: no context injection (default)
- `local`: load chunks from a local text/markdown document and inject top matches into prompts
- `semantic`: feature-gated semantic ranking backend (`--features semantic-search`)

Example:

```bash
cargo run -- \
  --retrieval-backend local \
  --retrieval-doc-path ./docs/knowledge.md \
  --retrieval-max-chunks 3 \
  --retrieval-max-chars 3000 \
  --retrieval-min-score 1 \
  ask "Create a release plan from our internal standards"
```

Feature-gated semantic backend:

```bash
cargo run --features semantic-search -- \
  --retrieval-backend semantic \
  --retrieval-doc-path ./docs/knowledge.md \
  ask "What are our rollout guardrails?"
```

Retrieval integration points:
- `ask`, `workflow`, `release-plan`: prompt is enriched before runner execution
- `chat`: each user turn is enriched before streaming execution

## Persistent Sessions

Use SQLite-backed sessions to persist conversation history across CLI restarts:

```bash
cargo run -- \
  --session-backend sqlite \
  --session-db-url sqlite://.zavora/sessions.db \
  --session-id team-planning \
  chat
```

Provider/model switching behavior:
- Today: switch provider/model per invocation with `--provider` and `--model`.
- Chat supports in-session switching via `/provider <name>`, `/model <id>`, and `/status`.
- Model/provider compatibility checks are enforced before switching.
- If a switch fails validation or runner rebuild, the previous provider/model and session remain active.

## Release Model

This repo follows a release train model with SemVer tags:

- Plan work by release slices (`R1`, `R2`, `R3`)
- Merge continuously behind CI
- Tag stable increments as `vX.Y.Z`
- Publish release notes from `CHANGELOG.md`

See `docs/AGILE_RELEASE_CYCLE.md` for the full process.
See `docs/PROJECT_PLAN.md` and `docs/GITHUB_MILESTONE_ISSUES.md` for the sprint roadmap and ticket breakdown.
See `docs/ADK_CAPABILITY_MATRIX.md`, `docs/ADK_TARGET_ARCHITECTURE.md`, and `docs/SPRINT_BACKLOG_RISK_REGISTER.md` for Sprint 0 execution artifacts.
See `docs/RETRIEVAL_ABSTRACTION.md` for retrieval interface and integration details.
See `docs/MCP_TOOLSET_MANAGER.md` for MCP profile schema, discovery, and runtime registration flow.
See `docs/GRAPH_WORKFLOWS.md` for reusable templates and graph routing behavior.
See `docs/EVAL_BASELINE.md` for current eval dataset baseline metrics.
