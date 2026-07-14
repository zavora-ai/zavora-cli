# Zavora CLI v2 design

## Runtime shape

```text
Developer
   │
   ▼
Zavora worker agent ───────────────► tools, MCP, files, shell, sessions
   │
   └── complex task ──► plan_work AgentTool ──► planner agent
                                                strong model
                                                no mutation tools
```

The worker owns the conversation and all product tools. The planner is intentionally narrow: it turns a complex goal and known constraints into an executable plan. ADK-Rust 2.0's `AgentTool` preserves typed agent composition without introducing a second application runtime.

## Default routing

| Role | Provider | Model | Pool | Policy |
| --- | --- | --- | --- | --- |
| Worker | OpenAI | `gpt-5.4-mini-2026-03-17` | 10M | Used for normal turns and tool work |
| Planner | OpenAI | `gpt-5.6-sol` | 1M | Used only when planning materially helps; maximum four calls per process |

The CLI reports locally observed calls and estimates only. OpenAI remains authoritative for actual token accounting and limits.

## Configuration precedence

CLI flag → environment variable → selected agent/profile → role default.

Legacy `--provider` and `--model` remain aliases for the worker role. New explicit flags are `--worker-provider`, `--worker-model`, `--planner-provider`, `--planner-model`, and `--planner-call-budget`.

## Terminal experience

The initial screen uses a compact identity line and a bordered runtime card rather than a large ASCII wordmark. It shows the active workspace and session, then two visible model-role rows. Tool activity remains streamed inline, while strong-model planning is labelled separately. `NO_COLOR` and non-TTY output use plain ASCII.

The first v2 release keeps the reliable line editor and command history. Its presentation layer is isolated so a future full-screen Ratatui shell can consume the same runtime events without changing agent execution.

## Safety and budget controls

- The planner receives no file-writing or shell tools.
- A `BudgetedPlannerTool` wraps `AgentTool` and rejects calls after the configured per-process ceiling.
- No model is called to generate a greeting.
- Model catalog metadata labels the shared daily pool rather than implying a separate limit for each model.
- Switching roles rebuilds the runner but keeps the same ADK session service.
