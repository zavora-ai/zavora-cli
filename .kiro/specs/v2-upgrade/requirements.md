# Zavora CLI v2 requirements

> **Superseded by [`.kiro/specs/v2-vision`](../v2-vision/requirements.md) on 2026-08-15.**
> That spec is the authoritative v2 contract and holds the reconciled backlog.
> This document is retained for provenance. Checkbox state here was reconciled
> against the runtime on 2026-08-15; where the implementation deliberately
> diverged, the divergence is recorded in v2-vision task 23.4.

## Intent

Zavora CLI v2 is a production-oriented terminal agent built on ADK-Rust 2.0. It should feel calm and legible during long coding sessions, spend scarce model capacity deliberately, and let developers understand which model is planning and which model is executing.

## Requirements

1. The crate must compile against the local ADK-Rust 2.0 workspace with Rust 1.94 or newer.
2. OpenAI must be the default provider. A fresh configuration uses `gpt-5.4-mini-2026-03-17` for routine work and `gpt-5.6-sol` for planning.
3. Planner and worker roles must have independent provider and model settings through CLI flags, profile configuration, environment variables, and interactive chat commands.
4. Other supported providers must remain selectable. Switching the worker must preserve the current session and must not silently replace the planner configuration.
5. The planner is a callable ADK-Rust `AgentTool`, invoked for complex planning rather than every turn. Its local call budget defaults to four calls per process and can be configured.
6. The OpenAI model catalog must reflect the models available to the project, distinguish the 1M and 10M daily token pools, recommend appropriate roles, and exclude the retired GPT-4.5 preview from active selection.
7. Startup must not spend tokens on a generated greeting. The interface must immediately show workspace, session, worker, planner, safety mode, and the commands needed to begin.
8. Interactive chat must provide `/models`, `/worker`, and `/planner` controls in addition to the existing provider/model commands.
9. Terminal output must remain readable without color, in narrow terminals, in redirected output, and with `NO_COLOR` set.
10. Documentation must explain the routing model, quota caveats, configuration, v2 migration, and verification commands.
11. Tests must cover defaults, role resolution, model catalog metadata, command parsing, planner budgeting, and provider validation without making paid model calls.

## Acceptance criteria

- `cargo fmt --all -- --check`, `cargo check --all-targets`, `cargo test --all-targets`, and Clippy pass.
- `zavora-cli --help`, `zavora-cli models`, and the chat startup surface correctly describe the two model roles.
- A deterministic mock test proves that the worker can call the bounded planner agent tool.
- The repository is committed with no untracked implementation files.
