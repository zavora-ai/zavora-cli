# Zavora CLI

[![Crates.io](https://img.shields.io/crates/v/zavora-cli.svg)](https://crates.io/crates/zavora-cli)
[![License: MIT](https://img.shields.io/badge/license-MIT-blue.svg)](LICENSE)

Zavora is an ADK-Rust 2.0 coding agent for the terminal. It keeps routine implementation work on an efficient worker model and brings in a stronger planning model only when a task needs architectural reasoning or a material replan.

```text
Developer ──► worker agent ──► files · shell · MCP · sessions
                    │
                    └── complex work ──► bounded planner agent
```

The full-screen terminal workspace is designed for long development sessions: streamed Markdown and code, retained tool activity, context usage, model-role status, multiline editing, and in-place approvals for consequential actions. It adapts from a side-by-side wide layout to a compact stacked layout without losing the active conversation.

## Install

Zavora v2 requires Rust 1.95 or newer when building from source.

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

An interactive terminal opens the retained workspace automatically. The worker route, active agent, BUILD or PLAN mode, conversation, tool runs, and composer remain visible as the session progresses. No tokens are spent generating a greeting.

```text
Shift+Tab   switch BUILD / PLAN
Enter       send
Ctrl+J      add a line
PageUp/Down review the conversation
Ctrl+End    follow the newest response
Ctrl+P      open keyboard help
Esc         request cancellation
```

For redirected output, a dumb terminal, or the complete line-oriented slash-command shell, use:

```bash
ZAVORA_CLASSIC=1 zavora-cli chat
```

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

Inside the classic chat shell:

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
```

Worker precedence is CLI/environment, selected agent, selected profile, then role
defaults. The planner resolves independently and only from planner-scoped
settings — `--planner-provider`/`--planner-model`, `ZAVORA_PLANNER_*`, and
`planner_provider`/`planner_model` in the profile. It does **not** inherit the
worker provider or a bare `provider` setting, and defaults to OpenAI. If you move
the worker to another provider and want the planner to follow, set the planner
role explicitly.

The v1 `provider` and `model` settings are still accepted as worker aliases. New
setup runs store API keys in the OS credential vault rather than project TOML.

## Everyday commands

```bash
zavora-cli                              # interactive chat
zavora-cli ask "Explain this workspace" # one response
zavora-cli workflow parallel "Review the API and tests"
zavora-cli ralph "Implement the accepted issue"
zavora-cli agents list
zavora-cli capabilities list
zavora-cli skills list
zavora-cli skills search device
zavora-cli skills validate .agents/skills/repository-development
zavora-cli skills link ./my-skill
zavora-cli plugins list
zavora-cli plugins validate ./my-extension
zavora-cli plugins install https://github.com/example/my-extension.git
zavora-cli plugins doctor --json
zavora-cli sessions list
zavora-cli mcp catalog office
zavora-cli mcp add docx-mcp
zavora-cli mcp list
zavora-cli doctor
```

One-shot commands have a versioned automation surface with clean stdout,
repeatable file inputs, automatic piped stdin, stable exits, and both single
JSON and streaming JSONL output:

```bash
zavora-cli ask --output-format json --file README.md "Summarize this project"
git diff | zavora-cli ask --output-format stream-json "Review this patch"
```

See [`docs/HEADLESS.md`](docs/HEADLESS.md) for schemas, events, approvals, and
exit codes.

The full-screen workspace and classic shell share the same runtime commands. In the TUI, `Ctrl+P` opens a searchable command palette; slash commands autocomplete with `Tab`; `↑`/`↓` recall prompt history; `Shift+Tab` changes build/plan mode; and the mouse wheel scrolls the transcript. It includes live tool activity, in-place approvals, model/provider switching, session switching, checkpoints, compaction, Markdown export, cancellable workflows, and a direct shell mode with `!`. A discovered skill is directly invocable as `/<skill-name> [request]`.

See [`docs/TUI.md`](docs/TUI.md) for the interaction and command contract.

## Capability model

Zavora reports capability state from the live runtime rather than relying on a static prompt:

- `AGENTS.md`, `GEMINI.md`, and `CLAUDE.md` families supply additive, scope-resolved project instructions. Native `AGENTS.override.md` takes precedence in its directory; Gemini custom context names, Claude local/rules files, imports, deduplication, and inspection are supported.
- `.agents/skills/<name>/SKILL.md` is the preferred portable skill layout. Compatible `.zavora`, Claude, Gemini, Grok, and OpenCode skill roots are discovered with deterministic precedence. Skills support install/link/update/enable/disable/uninstall and are injected through ADK-Rust's plugin runtime for every model provider.
- Plugins normalize native Zavora, Codex, Claude, Gemini, Grok, and OpenCode packages. Enabled packages contribute namespaced skills, portable Markdown agents/commands, and MCP servers. JavaScript/TypeScript entrypoints are reported but require an explicit trusted runtime; discovery never silently executes package code.
- Specialist subagents cover productivity artifacts, development, research, operations/devices, and independent review.
- Capability recipes describe useful MCP server combinations. `configured`, `connected`, and `authorized` are reported separately; enabling a recipe never claims that its servers are usable.

Use `zavora-cli capabilities list`, `zavora-cli skills list`, `zavora-cli agents list`, and `zavora-cli mcp doctor` for progressively deeper inspection.

The dated competitor and gap assessment lives in [`docs/CLI_CAPABILITY_MATRIX.md`](docs/CLI_CAPABILITY_MATRIX.md).
The exact instruction discovery and precedence contract is documented in [`docs/PROJECT_INSTRUCTIONS.md`](docs/PROJECT_INSTRUCTIONS.md).

## Tools and operating controls

The standard runtime includes workspace-aware file reading and editing, glob and ripgrep search, shell execution, GitHub operations, todos, time, memory, release planning, and tool discovery. Optional features add web fetching, LSP, browser automation, sandboxed code execution, and RAG.

```bash
cargo install zavora-cli --features "web-fetch,lsp,oauth,browser,sandbox,rag"
```

MCP works in both directions:

- As a client, Zavora discovers tools from configured stdio or HTTP MCP servers. `mcp catalog`, `mcp add`, and `mcp remove` provide comment-preserving configuration for curated productivity, development, research, device, and registry servers.
- As a server, `zavora-cli mcp serve` exposes the built-in tool surface over stdio.

Stdio tool discovery prefers the MCP `2026-07-28` `server/discover` lifecycle, falls back to `2025-11-25` for compliant legacy servers, advertises tool-call task support, and bounds the handshake by the server timeout. `zavora-cli mcp protocol --json` reports remaining transport and authorization gates instead of overstating compatibility.

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
