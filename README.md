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

## Development Commands

Use `make` targets:

- `make fmt`
- `make fmt-check`
- `make check`
- `make lint`
- `make test`
- `make ci`
- `make release-check`

## Persistent Sessions

Use SQLite-backed sessions to persist conversation history across CLI restarts:

```bash
cargo run -- \
  --session-backend sqlite \
  --session-db-url sqlite://.zavora/sessions.db \
  --session-id team-planning \
  chat
```

## Release Model

This repo follows a release train model with SemVer tags:

- Plan work by release slices (`R1`, `R2`, `R3`)
- Merge continuously behind CI
- Tag stable increments as `vX.Y.Z`
- Publish release notes from `CHANGELOG.md`

See `docs/AGILE_RELEASE_CYCLE.md` for the full process.
See `docs/PROJECT_PLAN.md` and `docs/GITHUB_MILESTONE_ISSUES.md` for the sprint roadmap and ticket breakdown.
