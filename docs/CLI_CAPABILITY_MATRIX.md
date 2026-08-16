# Agent CLI capability matrix

This initial, capability-by-capability comparison is the canonical product roadmap for Zavora CLI.

Status date: 2026-08-15. `✓` means the capability is usable, `◐` means partial or preview, and `—` means it is not implemented in Zavora yet. Competitor entries summarize documented product surfaces, not implementation equivalence.

| Capability | Gemini CLI | OpenCode | Codex CLI | Grok Build CLI | Zavora CLI today |
|---|---|---|---|---|---|
| Primary position | Google coding agent | Provider-neutral coding platform | Integrated coding/work agent | Extensible coding agent | ADK-Rust agent platform |
| Model providers | Gemini/Vertex | ✓ 75+ and local | Primarily OpenAI; managed Bedrock | xAI plus custom compatible models | ✓ OpenAI, Gemini, Anthropic, DeepSeek, Groq, Ollama |
| Interactive TUI | ✓ polished | ✓ polished | ✓ polished | ✓ rich mouse TUI | ✓ searchable command palette, full chat-command parity, history/completion, live tool activity, approvals, sessions/checkpoints/export, route switching, build/plan/shell modes, cancellation, and responsive mouse/keyboard navigation |
| Headless automation | ✓ text/JSON/stream-JSON | ✓ JSON, server, attach | ✓ exec, JSONL, SDK/app server | ✓ JSON/streaming JSON | ✓ versioned text/JSON/JSONL contract across ask, workflows, release plans, named agents, and Ralph; composed stdin/files, clean stdout, stable exits, safe approvals, buffered output guards |
| Project instructions | `GEMINI.md`; configurable `AGENTS.md` | ✓ `AGENTS.md` | ✓ hierarchical `AGENTS.md` | ✓ `AGENTS.md` plus Claude compatibility | ✓ additive root-to-CWD `AGENTS.md`, `GEMINI.md`, and `CLAUDE.md` families; overrides/local files, configurable Gemini names, unscoped Claude rules with scoped-rule deferral, safe imports, deduplication, limits, CLI/TUI inspection |
| Standard `SKILL.md` | ✓ progressive loading | ✓ native on-demand | ✓ progressive loading | ✓ standard plus Claude skills | ✓ `.agents`, Zavora, Claude, Gemini, Grok, and OpenCode discovery; deterministic precedence, plugin namespacing, ADK runtime injection, direct invocation |
| Skill management | ✓ install/link/update/enable/disable | ◐ discovery/loading | ✓ skills plus plugin distribution | ✓ skills plus plugins/marketplaces | ✓ validate/install/link/update/enable/disable/uninstall at workspace and user scope, with managed copies and live linked development |
| Custom subagents | ✓ Markdown definitions, tool isolation | ✓ Markdown agents and permissions | ✓ custom agents and thread UI | ✓ agents and independent sessions | ◐ TOML agents plus five built-in specialists |
| Parallel subagents | ✓ local and remote | ✓ task-based parallel work | ✓ visible concurrent agent threads | ✓ parallel child sessions | ◐ ADK parallel workflow, with limited user control and visibility |
| MCP transports | ✓ stdio, SSE, Streamable HTTP | ✓ stdio/remote | ✓ stdio/Streamable HTTP | ✓ stdio/HTTP | ✓ stdio/Streamable HTTP |
| MCP resources/prompts | ✓ first-class UI and `@resource` | ✓ tools, prompts, instructions | ✓ tools and server instructions | ✓ integrated MCP management | ✓ commands for resources and prompts |
| MCP OAuth | ✓ managed OAuth | ✓ remote OAuth | ✓ bearer/OAuth/session auth | ✓ auth and doctor workflow | ◐ optional compile feature |
| MCP 2026 completeness | Strong modern implementation | Strong V2 implementation | Strong managed implementation | Strong managed implementation | ◐ native `2026-07-28` stdio discovery and negotiated tool-call tasks; HTTP discovery lifecycle, interactive elicitation, authorization validation, and MRTR resume remain gated |
| Plugins/extensions | ✓ installable extensions | ◐ npm/TypeScript plugin API; V2 beta | ✓ universal plugin directory | ✓ plugins and marketplaces | ✓ normalized Zavora/Codex/Claude/Gemini/Grok/OpenCode discovery and manifests; validate/install/link/update/enable/disable/uninstall/doctor; skills, portable Markdown agents/commands, and MCP runtime contributions. Executable JS/TS requires an explicit trusted runtime |
| Hooks | ✓ lifecycle hooks | ✓ plugin hooks | ✓ comprehensive trusted hooks | ✓ project/plugin hooks | ◐ executor exists but is not wired into the runtime |
| Permissions/policy | ✓ policy engine plus sandbox | ✓ granular agent/tool rules | ✓ sandbox, profiles, approvals | ✓ modes, rules, and sandbox | ◐ confirmations and guardrails; no unified policy engine |
| Sessions/checkpoints | ✓ resume and automatic checkpoints | ✓ resume/fork/share/export | ✓ resume/fork/worktrees | ✓ resume/fork/worktrees/export | ✓ SQLite sessions and persisted conversation checkpoints |
| ACP/IDE protocol | ✓ `gemini --acp` | ✓ `opencode acp` | App Server/IDE integration | ✓ `grok agent stdio` | — ADK-Rust has `adk-acp`, but Zavora does not expose it |
| Remote A2A agents | ✓ | — | — | — | ✓ A2A server foundation; weak discovery/client UX |
| Artifact creation | Primarily extensions/MCP | Primarily skills/MCP | Strong plugin and artifact ecosystem | Broad Build capabilities | ◐ DOCX/PPTX/XLSX/PDF skills and MCP recipes exist, but bundled execution assets are incomplete |
| Email/calendar/apps | Extensions/MCP | MCP/plugins | Plugins/connectors | Plugins/MCP | ◐ catalog recipes only; configured is not connected or authorized |
| Device/computer management | Extensions/MCP | MCP/plugins | Computer-use/plugin surfaces | Plugins/MCP | ◐ catalog recipes only; configured is not connected or authorized |
| Self-inspection | `/about`, `/tools`, `/agents`, `/skills`, `/mcp` | Config/agents/tools views | Rich runtime inspection | ✓ `grok inspect` | ✓ live capability snapshot shared by `/inspect`, `/doctor`, `/capabilities`, and the default-agent prompt |
| Evals/telemetry/A2A server | ◐ | ◐ | Strong product telemetry | Strong enterprise controls | ✓ unusually strong ADK foundation |

## Implemented in this milestone

1. Standards-first `SKILL.md` discovery and scoped `AGENTS.md` resolution.
2. A live capability registry shared by CLI, classic chat, TUI, and the default-agent prompt.
3. Curated, actionable MCP recipes for documents, slides, spreadsheets, PDF, email, research, development, devices, computer use, and registry operations.
4. MCP lifecycle commands with configuration-preserving edits, diagnostics, OAuth entry point, resources, prompts, and explicit protocol reporting.
5. Five bounded specialist subagents and capability-aware tool routing.

## Next maturity gates

1. Add marketplace/registry search, signed package verification, lockfiles, and policy-controlled executable plugin runtimes.
2. Add per-agent skill permissions and hot reload inside a running session.
3. Wire declarative plugin hooks into runner/tool events with trust prompts and `/hooks` inspection.
4. Expose ADK-Rust ACP as a supported CLI host command and test editor handoff.
5. Close the remaining MCP 2026 gates reported by `zavora-cli mcp protocol --json` before calling protocol support complete.

## Sources

- [Gemini CLI command reference](https://geminicli.com/docs/cli/commands/), [skills](https://geminicli.com/docs/cli/using-agent-skills/), and [MCP](https://geminicli.com/docs/tools/mcp-server/)
- [OpenCode commands](https://opencode.ai/docs/commands/), [skills](https://opencode.ai/docs/skills), [agents](https://opencode.ai/docs/agents), and [plugins](https://opencode.ai/v2/docs/build/plugins)
- [Codex CLI slash commands](https://learn.chatgpt.com/docs/developer-commands.md?surface=cli), [AGENTS.md](https://learn.chatgpt.com/docs/custom-instructions.md), and [MCP](https://learn.chatgpt.com/docs/extend/mcp.md)
- [Grok Build skills, plugins, and marketplaces](https://docs.x.ai/build/features/skills-plugins-marketplaces) and [CLI reference](https://docs.x.ai/build/cli/reference)
- [Claude Code plugin reference](https://code.claude.com/docs/en/plugins-reference)
- [Gemini CLI extension reference](https://geminicli.com/docs/extensions/reference/)
