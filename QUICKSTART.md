# Zavora CLI v2 quickstart

## 1. Install and configure

Zavora CLI v2 requires Rust 1.95+ when installed from source. The pinned
toolchain is declared in `rust-toolchain.toml`.

```bash
cargo install zavora-cli
zavora-cli setup
```

Setup defaults to OpenAI and stores the API key in the operating-system credential vault. You can use an environment variable instead:

```bash
export OPENAI_API_KEY="..."
```

## 2. Inspect model routing

```bash
zavora-cli models
```

The default worker, `gpt-5.4-mini-2026-03-17`, handles normal conversation, code, and tools. The stronger `gpt-5.6-sol` planner is exposed to the worker as the bounded `plan_work` agent tool. It is intended for multi-file changes, architectural decisions, and material replanning, with a default maximum of four calls per process.

## 3. Start in a project

```bash
cd your-project
zavora-cli
```

Try:

```text
Explain this workspace and show me its main execution path.
Add validation to the user registration endpoint and run the relevant tests.
Plan the database migration first, then wait for my approval.
```

Zavora streams Markdown and tool activity as it works. File writes, shell commands, GitHub changes, and external MCP tools are subject to the configured permission and confirmation rules.

## 4. Control the two roles

Inside chat:

```text
/status
/models
/worker gpt-5.4-mini-2026-03-17
/planner gpt-5.6-sol
/planner-provider openai
```

From the shell:

```bash
zavora-cli \
  --worker-provider openai \
  --worker-model gpt-5.4-mini-2026-03-17 \
  --planner-provider openai \
  --planner-model gpt-5.6-sol \
  --planner-call-budget 2 \
  chat
```

Provider and model switches preserve the current session. Other supported providers are OpenAI, Anthropic, Gemini, DeepSeek, Groq, and Ollama.

## 5. Preserve context

Useful controls during a longer session:

```text
/usage                       inspect context consumption
/compact                     compact older conversation events
/checkpoint save before-api  save a restorable point
/checkpoint list             list checkpoints
/tangent                     explore without losing the main thread
/memory remember <text>      retain a useful project fact
```

Use SQLite when conversations must survive restarts:

```toml
[profiles.default]
session_backend = "sqlite"
session_db_url = "sqlite://.zavora/sessions.db"
```

## 6. Connect MCP capabilities

Configure a local stdio server:

```toml
[[profiles.default.mcp_servers]]
name = "project-tools"
command = "path/to/server"
args = []
enabled = true
```

Then inspect it:

```bash
zavora-cli mcp list
zavora-cli mcp discover --server project-tools
```

Zavora can also expose its own tools as an MCP server with `zavora-cli mcp serve`.

## 7. Verify an installation

These commands do not call a paid model:

```bash
zavora-cli --help
zavora-cli models
zavora-cli doctor
```

See [README.md](README.md) for profiles, optional features, permissions, server mode, and development checks.
