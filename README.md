# Zavora CLI

[![Crates.io](https://img.shields.io/crates/v/zavora-cli.svg)](https://crates.io/crates/zavora-cli)
[![License: MIT](https://img.shields.io/badge/license-MIT-blue.svg)](LICENSE)

Zavora is an ADK-Rust 2.0 coding agent for the terminal. It keeps routine implementation work on an efficient worker model and brings in a stronger planning model only when a task needs architectural reasoning or a material replan.

```text
Developer ──► worker agent ──► files · shell · MCP · sessions
                    │
                    └── complex work ──► bounded planner agent
```

The interface is designed for long development sessions: streamed Markdown, visible tool activity, command history, checkpoints, context usage, model-role status, and confirmations for consequential actions. It works in narrow terminals, suppresses styling in redirected output, and honours `NO_COLOR`.

## Install

Zavora v2 requires Rust 1.94 or newer when building from source.

```bash
cargo install zavora-cli
```

Prebuilt release wrappers are also available:

```bash
npm i -g @zavora-ai/zavora-cli
brew install --formula https://raw.githubusercontent.com/zavora-ai/zavora-cli/main/Formula/zavora-cli.rb
```

## Start

OpenAI is the default provider. Store a key in the operating-system credential vault:

```bash
zavora-cli setup
zavora-cli chat
```

Or provide it for the current shell:

```bash
export OPENAI_API_KEY="..."
zavora-cli chat
```

The startup card shows both active roles before a model is called:

```text
ZAVORA v2.0.0 · ADK-Rust 2.0
workspace  my-project  session  default-session
┌─ MODEL ROUTING
│ WORKER   Openai/gpt-5.4-mini-2026-03-17
│ PLANNER  Openai/gpt-5.6-sol · max 4 calls
└────────────────────────────────────────────
```

No tokens are spent generating a greeting.

## Planner and worker routing

The worker owns the conversation, tools, and implementation. By default it uses `gpt-5.4-mini-2026-03-17`, from the larger shared daily token pool. The planner is an ADK-Rust `AgentTool` backed by `gpt-5.6-sol`. It has no mutation tools and is available only to produce a concise plan for complex work.

The planner call budget defaults to four calls for one CLI process. It is a local guardrail, not a measurement of provider-side token usage. OpenAI remains authoritative for account limits.

```bash
# Show available models, recommended roles, and shared quota pools
zavora-cli models

# Set roles independently
zavora-cli \
  --worker-provider openai \
  --worker-model gpt-5.4-mini-2026-03-17 \
  --planner-provider openai \
  --planner-model gpt-5.6-sol \
  --planner-call-budget 3 \
  chat
```

Inside chat:

```text
/models                         show the catalog and quota pools
/worker [model]                 switch the everyday model
/planner [model]                switch the planning model
/provider <name>                switch the worker provider
/planner-provider <name>        switch the planner provider
/status                         show the resolved runtime
```

Changing either role rebuilds the agent while preserving the current ADK session service.

## Other providers

OpenAI, Anthropic, Gemini, DeepSeek, Groq, and Ollama remain supported. The worker and planner can use different providers when both credentials are available.

```bash
# Anthropic worker with an OpenAI planner
zavora-cli \
  --worker-provider anthropic \
  --worker-model claude-sonnet-4-20250514 \
  --planner-provider openai \
  --planner-model gpt-5.6-sol \
  chat

# Fully local
zavora-cli \
  --worker-provider ollama --worker-model llama4 \
  --planner-provider ollama --planner-model qwen2.5-coder \
  chat
```

Credential environment variables are `OPENAI_API_KEY`, `ANTHROPIC_API_KEY`, `GOOGLE_API_KEY`, `DEEPSEEK_API_KEY`, and `GROQ_API_KEY`. Ollama uses `OLLAMA_HOST`, defaulting to `http://localhost:11434`.

## Profile configuration

Role-specific settings live in `.zavora/config.toml`:

```toml
[profiles.default]
worker_provider = "openai"
worker_model = "gpt-5.4-mini-2026-03-17"
planner_provider = "openai"
planner_model = "gpt-5.6-sol"
planner_call_budget = 4

session_backend = "sqlite"
session_db_url = "sqlite://.zavora/sessions.db"
tool_confirmation_mode = "mcp-only"
auto_compact_enabled = true
```

Precedence is CLI/environment, selected agent, selected profile, then role defaults. The v1 `provider` and `model` settings are still accepted as worker aliases. New setup runs store API keys in the OS credential vault rather than project TOML.

## Everyday commands

```bash
zavora-cli                              # interactive chat
zavora-cli ask "Explain this workspace" # one response
zavora-cli workflow parallel "Review the API and tests"
zavora-cli ralph "Implement the accepted issue"
zavora-cli agents list
zavora-cli skills list
zavora-cli sessions list
zavora-cli mcp list
zavora-cli doctor
```

Useful chat controls include `/help`, `/tools`, `/mcp`, `/usage`, `/compact`, `/checkpoint`, `/tangent`, `/todos`, `/delegate`, `/agent`, and `/exit`.

## Tools and operating controls

The standard runtime includes workspace-aware file reading and editing, glob and ripgrep search, shell execution, GitHub operations, todos, time, memory, release planning, and tool discovery. Optional features add web fetching, LSP, browser automation, sandboxed code execution, and RAG.

```bash
cargo install zavora-cli --features "web-fetch,lsp,oauth,browser,sandbox,rag"
```

MCP works in both directions:

- As a client, Zavora discovers tools from configured stdio or HTTP MCP servers and can update the configured server set.
- As a server, `zavora-cli mcp serve` exposes the built-in tool surface over stdio.

Write, shell, GitHub, and externally supplied MCP tools pass through confirmation and permission policy. The system prompt never commits or pushes unless the developer asks.

## ADK-Rust 2.0 architecture

Zavora uses the v2 `Runner`, typed events, `AgentTool`, session services, tool traits, model clients, skills, memory, guardrails, telemetry, MCP integration, A2A server support, and compaction APIs. The old `adk-ralph` 0.5 dependency and duplicate ADK runtime graph have been removed; Ralph now runs through the same v2 worker/planner runtime as chat.

See the implementation specification in [`.kiro/specs/v2-upgrade/`](.kiro/specs/v2-upgrade/) and the v1 migration notes in [`docs/MIGRATION_GUIDE_v2.md`](docs/MIGRATION_GUIDE_v2.md).

## Development

```bash
cargo fmt --all -- --check
cargo check --all-targets
cargo test --all-targets -- --test-threads=1
cargo clippy --all-targets -- -D warnings

# CLI smoke checks; these do not call a paid model
cargo run -- --help
cargo run -- models
NO_COLOR=1 cargo run -- models
```

The test suite is deliberately deterministic and uses ADK-Rust mock models for runtime tests.

## License

MIT
