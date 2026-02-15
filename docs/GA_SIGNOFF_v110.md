# GA Sign-Off (`v1.1.0-rc`)

Sign-off date: **February 15, 2026**

## Scope

- Milestone: Phase 2 — Q CLI Parity + UX (Sprints 9–11)
- Issues: #37–#47 (11 issues across 3 sprints)

## Phase 2 Deliverables Checklist

### Sprint 9 — Tool Policy and Diagnostics
- [x] Tool aliases with wildcard allow/deny filtering (#37, PR #49)
- [x] Hook lifecycle system with 5 hook points (#38, PR #50)
- [x] MCP diagnostics with server state and auth hints (#39, PR #51)

### Sprint 10 — Context Management
- [x] Context usage tracking with budget warnings (#40, PR #52)
- [x] Manual /compact and auto-compaction via ADK (#41, PR #53)
- [x] Checkpoint save/list/restore and tangent mode (#42, PR #54)
- [x] Todo list persistence and delegate experiment (#43, PR #55)

### Sprint 11 — UX Polish and Release
- [x] Unified theme, command palette, fuzzy matching, onboarding (#44, PR #56)
- [x] Parity benchmark suite with 12 scenarios and scorecard (#45, PR #57)
- [x] Parity matrix (98.5%) and differentiation roadmap (#46, PR #58)
- [x] v1.1.0 RC release plan and sign-off (#47, this PR)

## Release Gates

| Gate | Status | Evidence |
|------|--------|----------|
| cargo test | ✓ Pass | 157 tests, 0 failures |
| cargo check (warnings) | ✓ Pass | 0 warnings |
| Parity score | ✓ Pass | 98.5% (threshold: 75%) |
| Security baseline | ✓ Pass | No new advisories |
| Backward compatibility | ✓ Pass | No breaking changes from v1.0.0 |

## Rollback Playbook

All Phase 2 features are additive — no breaking changes to v1.0.0 behavior:

1. **Chat commands**: New slash commands (/compact, /checkpoint, /tangent, /todos, /delegate) are opt-in. Existing commands unchanged.
2. **Auto-compaction**: Enabled by default but configurable via `auto_compact_enabled: false` in profile config.
3. **Hook system**: Only active when `hooks` are configured in agent catalog. No hooks = no behavior change.
4. **Tool aliases**: Only active when `allow_tools`/`deny_tools` are configured. Default = all tools allowed.
5. **Theme/prompt**: Prompt changes are visual only. No functional impact on input parsing.

To rollback: revert to v1.0.0 tag. No data migration required.

## New Capabilities Summary

| Feature | Command/Config | Module |
|---------|---------------|--------|
| Tool aliases | `allow_tools`, `deny_tools` in agent catalog | tool_policy.rs |
| Hook lifecycle | `hooks` in agent catalog | hooks.rs |
| MCP diagnostics | `/mcp` in chat | mcp.rs |
| Context usage | `/usage` in chat | context.rs |
| Compaction | `/compact` in chat, `auto_compact_enabled` | compact.rs |
| Checkpoints | `/checkpoint save\|list\|restore` | checkpoint.rs |
| Tangent mode | `/tangent`, `/tangent tail` | checkpoint.rs |
| Todo lists | `/todos` in chat | todos.rs |
| Delegate | `/delegate` (experimental) | todos.rs |
| Command palette | Fuzzy matching on unknown commands | theme.rs |
| Onboarding | Auto-detected on first run | theme.rs |
| Benchmark suite | `src/benchmark.rs`, `docs/PARITY_BENCHMARK.md` | benchmark.rs |

## Migration Guidance

### From v1.0.0 to v1.1.0

No breaking changes. All new features are additive.

**Optional configuration additions:**

```toml
[profiles.default]
# Auto-compaction (default: enabled)
# auto_compact_enabled = true
# compact_interval = 10
# compact_overlap = 2
```

**New agent catalog fields:**

```toml
[agents.default]
allow_tools = ["fs_read", "fs_write", "execute_bash"]
deny_tools = ["execute_bash.rm_*"]

[agents.default.hooks]
pre_tool = [{ command = "echo pre", matcher = "execute_bash" }]
```

**New slash commands in chat:**
- `/compact` — summarize conversation to free context
- `/checkpoint save|list|restore` — manage conversation snapshots
- `/tangent` — enter/exit exploratory branch
- `/todos` — view/manage task lists
- `/delegate <task>` — (experimental) isolated sub-agent task
- `/usage` — now shows token breakdown (was help text)

## Residual Risks

- Delegate sub-agent runner not yet wired (data model only) — tracked for v1.2.0
- Text-based compaction summarizer is simpler than LLM-based — upgrade planned for v1.2.0
- Checkpoint store is in-memory only — not persisted across CLI restarts
- Upstream RustSec advisories tracked in `.cargo/audit.toml` — review each release
