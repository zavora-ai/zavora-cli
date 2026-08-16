# Headless automation

Zavora exposes a versioned automation contract for one-shot prompts, workflows,
release plans, named agents, and Ralph development runs. Human-readable text
remains the default.

## Commands

```bash
zavora-cli ask "Summarize the repository"
zavora-cli ask --output-format json "Summarize the repository"
zavora-cli workflow parallel --output-format stream-json "Research the options"
zavora-cli release-plan --output-format json "Ship the beta"
zavora-cli agents run --name research_agent --output-format json "Compare sources"
zavora-cli ralph --output-format stream-json "Implement the accepted issue"
```

`--output-format` accepts `text`, `json`, and `stream-json`. `jsonl` and
`streaming-json` are aliases for `stream-json`. Structured output is written
only to stdout; logs and tool diagnostics use stderr.

When no subcommand is supplied, piped stdin or a structured output flag selects
one-shot `ask` behavior:

```bash
git diff | zavora-cli --output-format stream-json
```

## Inputs

Prompt text, repeated UTF-8 files, and stdin are composed in that order:

```bash
cat issue.md | zavora-cli ask --file Cargo.toml "Propose a fix"
```

- Piped stdin is consumed automatically.
- `--stdin` forces an stdin read, including from a terminal.
- `--no-stdin` disables automatic piped-stdin consumption.
- `--file PATH`/`-f PATH` may be repeated.
- Each stdin or file input is limited to 16 MiB, and the assembled prompt still
  respects the configured prompt-character limit.

## JSON result

`json` emits exactly one object followed by a newline:

```json
{
  "schema_version": "zavora.headless.v1",
  "type": "result",
  "success": true,
  "command": "ask",
  "session_id": "default-session",
  "provider": "openai",
  "model": "gpt-5",
  "response": "...",
  "stats": {
    "duration_ms": 431,
    "response_chars": 120,
    "tool_calls": 0
  }
}
```

## Streaming JSON

`stream-json` is newline-delimited JSON. Every line is flushed immediately and
contains `schema_version`, `sequence`, `timestamp`, `type`, `command`,
`session_id`, and `data`. Event types are:

- `init`
- `agent`
- `system`
- `message`
- `tool_use`
- `tool_result`
- `error`
- `result`

Output guardrails in `block` or `redact` mode force buffered streaming. Zavora
then emits `init` with `data.buffered: true`, one safe `message`, and `result`.
This prevents unsafe partial content from leaking before the guardrail runs.

## Errors and exits

Structured modes emit a versioned error object with category, message, hint,
and exit code. Exit codes are stable:

| Code | Meaning |
|---:|---|
| 0 | Success |
| 2 | CLI syntax error (Clap) |
| 10 | Provider/configuration failure |
| 11 | Session/storage failure |
| 12 | Tool, MCP, or retrieval failure |
| 42 | Invalid or missing input |
| 70 | Internal failure |

## Tool approvals

Automation never waits for an interactive approval. Read-only or explicitly
allowed tools can run; an unapproved mutating tool returns a tool error. Use a
narrow approval where possible:

```bash
zavora-cli ask --approve-tool fs_write "Create the requested file"
```

`--always-approve` (alias `--yolo`) explicitly approves every available tool
for the process. It is intended only for an already isolated environment.
