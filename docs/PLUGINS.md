# Skills and cross-CLI plugins

Zavora uses one provider-neutral extension runtime. OpenAI, Gemini, Anthropic, DeepSeek, Groq, and Ollama agents receive the same enabled skills and MCP tools because packages are resolved before the shared ADK-Rust runner is built.

## Skills

The preferred portable layout is `<root>/<name>/SKILL.md`. Zavora discovers project and user skills from:

- `.agents/skills`
- `.zavora/skills`
- `.claude/skills`
- `.gemini/skills`
- `.grok/skills`
- `.opencode/skills` and `~/.config/opencode/skills`
- legacy `.skills`
- every enabled plugin's skill roots

Use the lifecycle commands at workspace scope (the default) or `--scope user`:

```console
zavora-cli skills list --json
zavora-cli skills search email
zavora-cli skills info repository-development
zavora-cli skills validate ./skills/repository-development
zavora-cli skills install ./skills/repository-development
zavora-cli skills link ./skills/repository-development
zavora-cli skills update repository-development
zavora-cli skills disable repository-development
zavora-cli skills enable repository-development
zavora-cli skills uninstall repository-development
```

An install creates a managed copy. A link keeps the original directory live for development. Git URLs are shallow-cloned and can be updated with `skills update`. Validation enforces the portable kebab-case name and matching directory contract.

`skills search` reads the Zavora registry from `ZAVORA_SKILLS_REGISTRY`, a sibling `skills-registry` checkout, or `~/.zavora/registries/skills`. A registry skill can be installed directly by name, for example `zavora-cli skills install device-fleet-management`; local registry content is preferred and the declared Zavora GitHub repository is the fallback.

## Plugins and extensions

Zavora recognizes these package envelopes:

| Ecosystem | Manifest or discovery root | Runtime compatibility |
|---|---|---|
| Zavora | `.zavora-plugin/plugin.json` | Skills, Markdown agents/commands, and MCP active; hooks/apps inspected |
| Codex | `.codex-plugin/plugin.json` | Skills, Markdown agents/commands, and MCP active; other components inspected |
| Claude | `.claude-plugin/plugin.json`, or standard component directories | Skills, Markdown agents/commands, and MCP active; hooks inspected |
| Gemini | `gemini-extension.json` | Skills, Markdown agents, and MCP active; TOML commands/hooks inspected; `${extensionPath}` substitution |
| Grok | Claude-compatible package under `.grok/plugins` | Skills, Markdown agents/commands, and MCP active; other components inspected |
| OpenCode | `opencode.json`, `package.json`, `.opencode/skills` | Standard skills active; JS/TS entrypoints inspected but not implicitly executed |

Plugin skills are namespaced as `plugin-name:skill-name`. Plugin MCP server names use the same `plugin-name:server-name` convention. This prevents one package from silently replacing an unrelated workspace capability.

```console
zavora-cli plugins list
zavora-cli plugins info office-suite --json
zavora-cli plugins validate ./office-suite
zavora-cli plugins install ./office-suite
zavora-cli plugins link ./office-suite --scope user
zavora-cli plugins install https://github.com/example/office-suite.git
zavora-cli plugins update office-suite
zavora-cli plugins disable office-suite
zavora-cli plugins enable office-suite
zavora-cli plugins uninstall office-suite
zavora-cli plugins doctor --json
```

The interactive CLI and TUI expose `/skills` and `/plugins` for live inspection.

## Trust boundary

Discovery is read-only. Declarative skills and MCP configuration are normalized, but connecting to an MCP server remains subject to Zavora's normal confirmations, authorization, and tool policy. Package paths cannot escape the package root and copied packages cannot contain symlinks.

OpenCode npm modules and arbitrary JavaScript, TypeScript, shell hooks, binaries, and app code are not executed merely because a package was discovered or enabled. `plugins doctor` reports those entrypoints. A future trusted runtime can execute them only after explicit policy and integrity checks.
