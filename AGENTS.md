# Zavora CLI workspace instructions

## Scope

These instructions apply to the repository. A deeper `AGENTS.md` or `AGENTS.override.md` may add or override instructions for its directory tree.

## Development

- Preserve unrelated work in the working tree.
- Prefer focused, idiomatic Rust changes and reuse ADK-Rust v2 abstractions before adding parallel infrastructure.
- Keep skills, MCP connectivity, authorization, and catalog recipes as separate states. Never describe a configured or enabled integration as connected without runtime evidence.
- Treat `.agents/skills/<name>/SKILL.md` as executable capability guidance and this file as project policy.
- Run focused tests before the full suite. Use the Rust version declared by `rust-version` in `Cargo.toml`.
- Do not commit, push, publish, send messages, or perform device mutations unless the user requests the external action.

## Capability surfaces

- Keep CLI, classic chat, and TUI results consistent for capabilities, skills, agents, MCP status, inspection, and diagnostics.
- The default agent should receive a concise live capability summary and delegate bounded work to specialist agents when that materially improves the result.
- Skills must remain truthful about unavailable tooling and include verification steps for artifact creation or external writes.
