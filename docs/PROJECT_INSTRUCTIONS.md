# Project instructions

Zavora loads native `AGENTS.md` instructions and compatible Gemini CLI and
Claude Code instruction files into the default agent context. Instruction
discovery is model-independent: the same project rules apply when Zavora runs
OpenAI, Gemini, Anthropic, DeepSeek, Groq, or Ollama models.

## Resolution order

Instructions are additive. Broad sources load first and instructions closest
to the current working directory load last.

1. Zavora user defaults from `~/.zavora/AGENTS.override.md` or
   `~/.zavora/AGENTS.md`.
2. Gemini user context from `~/.gemini/<context-file>`.
3. Claude user context from `~/.claude/CLAUDE.md` and unscoped Markdown files
   under `~/.claude/rules/`.
4. Every directory from the repository root to the current directory. Within
   each directory Zavora loads:
   - Gemini context files.
   - `.claude/CLAUDE.md` and `CLAUDE.md`.
   - Unscoped `.claude/rules/**/*.md` files in lexical order.
   - `CLAUDE.local.md`.
   - `AGENTS.override.md`, or `AGENTS.md` when no override exists.

This makes native `AGENTS.md` the final instruction family at a directory
scope. `AGENTS.override.md` replaces `AGENTS.md` only in the directory where
the override exists; parent and child scopes remain additive.

## Gemini compatibility

`GEMINI.md` is loaded by default. Zavora also reads the effective
`context.fileName` string or array from `~/.gemini/settings.json` and the
repository `.gemini/settings.json`; project settings take precedence. Invalid
absolute or parent-traversing context names are ignored.

## Claude compatibility

Zavora supports:

- `CLAUDE.md`
- `.claude/CLAUDE.md`
- `CLAUDE.local.md`
- recursively discovered `.claude/rules/*.md`

Rules with `paths` frontmatter are reported as deferred. They are not injected
globally because doing so would broaden their intended scope. Dynamic loading
when a tool first enters a matching subtree is a separate runtime capability.

## Imports and limits

Existing files referenced with `@relative/path.md`, `@/absolute/path.md`, or
`@~/path.md` are expanded inline. Relative paths resolve from the containing
instruction file. Import traversal is cycle-safe and limited to five levels.
Canonical imports must remain inside the repository or the user's `.zavora`,
`.gemini`, or `.claude` configuration roots. External files and symlinks are
blocked and reported as warnings instead of silently sending their contents to
the model.

- Maximum individual instruction file: 64 KiB.
- Maximum combined resolved context: 256 KiB.
- Canonically identical files and symlinks are loaded once.
- Missing `@` references remain ordinary text.

## Inspection

```bash
zavora-cli instructions list
zavora-cli instructions list --json
zavora-cli instructions show
zavora-cli instructions show --json
```

Interactive chat and the TUI provide `/instructions` and
`/instructions show`. `/inspect` reports active and deferred instruction
counts. New agent runtimes resolve the files from disk, so no stale global
cache is used.
