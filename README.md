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
cargo run -- release-plan "Build an enterprise-ready AI CLI" --releases 3
```

```bash
cargo run -- doctor
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
- `make ci`
- `make release-check`

## Profiles

`zavora-cli` can resolve runtime defaults from profiles in `.zavora/config.toml` (override path with `--config`).

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

[profiles.ops]
provider = "anthropic"
model = "claude-sonnet-4-20250514"
session_backend = "sqlite"
session_db_url = "sqlite://.zavora/ops-sessions.db"
```

Inspect profile state:

```bash
cargo run -- profiles list
cargo run -- --profile ops profiles show
```

## Retrieval Abstraction

Retrieval is pluggable and disabled by default.

- `disabled`: no context injection (default)
- `local`: load chunks from a local text/markdown document and inject top matches into prompts

Example:

```bash
cargo run -- \
  --retrieval-backend local \
  --retrieval-doc-path ./docs/knowledge.md \
  --retrieval-max-chunks 3 \
  ask "Create a release plan from our internal standards"
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
