# Documentation Discrepancies Report

This document identifies claims made in the documentation that don't match the actual implementation.

## 1. Hook Configuration Location (README.md)

**Documented:**
```toml
[[profiles.default.hooks.pre_tool]]
name = "block-rm"
match_tool = "execute_bash"
match_args = "rm -rf"
action = "block"
message = "Destructive rm blocked by hook policy"
```

**Reality:**
- Hooks are NOT configured in profiles, they're configured in agent catalogs (`.zavora/agents.toml`)
- The configuration structure is different from what's documented
- Actual implementation in `src/config.rs` line 129: `hooks: HashMap<String, Vec<HookConfig>>`
- Hooks belong to `AgentFileConfig`, not `ProfileConfig`

**Actual Hook Config Structure (src/hooks.rs lines 48-58):**
```rust
pub struct HookConfig {
    pub command: String,
    pub timeout_ms: u64,
    pub max_output: usize,
    pub matcher: Option<String>,
}
```

The documented fields `name`, `match_tool`, `match_args`, `action`, and `message` don't exist in the actual implementation.

**Correct Configuration Should Be:**
```toml
[agents.default.hooks]
pre_tool = [
    { command = "...", timeout_ms = 30000, max_output = 10240, matcher = "execute_bash" }
]
```

## 1b. Compaction Configuration (README.md lines 136-137)

**Status: FIXED ✅**

**Previously Documented:**
```toml
compaction_threshold = 0.75
compaction_target = 0.50
```

**Was:** These fields didn't exist - only event-based compaction via `compact_interval`

**Now Implemented:**
- Added `compaction_threshold` (default 0.75 = 75% context usage)
- Added `compaction_target` (default 0.50 = 50% context usage)
- Auto-compaction now triggers when token usage exceeds threshold
- Compacts repeatedly until target utilization is reached
- New `/autocompact` command to toggle on/off

**Implementation:**
- `src/config.rs`: Added fields to RuntimeConfig and ProfileConfig
- `src/compact.rs`: Added `compact_to_target()` function
- `src/chat.rs`: Added auto-compaction check after each response
- `src/chat.rs`: Added `/autocompact` command

Documentation is now accurate.

## 2. Multi-Agent Orchestration Claims (README.md lines 75-81)

**Documented:**
```
The assistant automatically delegates to specialist sub-agents when appropriate:

- **git agent** — git operations, commits, branch management
- **research agent** — codebase exploration, file search, analysis
- **planner agent** — task breakdown, todo lists, project planning

Transfers are visible in the UI with `→ agent_name` indicators.
```

**Reality:**
- ✅ Sub-agents ARE implemented (`src/runner.rs` lines 253-335)
- ✅ Three specialist agents exist: `git_agent`, `research_agent`, `planner_agent`
- ✅ They are properly configured with appropriate tools
- ✅ UI indicators ARE implemented (`src/streaming.rs` lines 367-374): Shows `→ agent_name` when author changes
- ✅ Tool calls shown with `⚡ tool_name` indicator

**Status:** Fully accurate.

## 3. Built-in Tools Table (README.md lines 118-124)

**Documented:**
```
| Tool | Purpose |
|------|---------|
| `fs_read` | Read files and directories with workspace path policy |
| `fs_write` | Create, overwrite, append, or patch files (confirmation required) |
| `execute_bash` | Run shell commands with safety policy (read-only commands auto-approved) |
| `github_ops` | GitHub operations via `gh` CLI (issues, PRs, projects) |
| `todo_list` | Create/complete/view/list/delete task lists (persisted to `.zavora/todos/`) |
```

**Reality:**
- ✅ All five tools are implemented in `src/tools/mod.rs`
- ✅ `todo_list` tool exists (lines 79-90)
- ✅ Tool functionality matches descriptions

**Status:** Accurate.

## 4. Agent Catalogs (README.md lines 131-145)

**Documented:**
```toml
[agents.coder]
description = "Code-focused assistant"
provider = "openai"
model = "gpt-4.1"
tool_confirmation_mode = "always"
allow_tools = ["fs_read", "fs_write", "execute_bash"]
```

**Reality:**
- ✅ Agent catalogs are fully implemented
- ✅ Configuration structure matches documentation
- ✅ `load_resolved_agents()` in `src/config.rs` line 218
- ✅ Precedence: implicit `default` → global `~/.zavora/agents.toml` → local `.zavora/agents.toml`
- ✅ Commands: `zavora-cli agents list`, `agents show`, `agents select` all implemented

**Status:** Accurate.

## 5. MCP Integration (README.md lines 191-200)

**Documented:**
```toml
[[profiles.ops.mcp_servers]]
name = "ops-tools"
endpoint = "https://mcp.example.com/ops"
enabled = true
timeout_secs = 15
auth_bearer_env = "OPS_MCP_TOKEN"
tool_allowlist = ["search_incidents", "get_runbook"]
```

**Reality:**
- ✅ MCP server configuration is implemented
- ✅ `McpServerConfig` in `src/config.rs` lines 161-171
- ✅ All documented fields exist: `name`, `endpoint`, `enabled`, `timeout_secs`, `auth_bearer_env`, `tool_allowlist`
- ✅ Additional field `tool_aliases` also exists

**Status:** Accurate.

## 6. Chat Commands (README.md lines 86-104)

**Documented:**
```
| `/delegate <task>` | Run isolated sub-agent prompt |
```

**Reality:**
- ✅ `/delegate` command is implemented (`src/chat.rs` line 91)
- ✅ `run_delegate()` function exists in `src/todos.rs` lines 207-252
- ✅ Creates isolated session and runs task
- ⚠️ Marked as "experimental" in code comments but not in README

**Status:** Accurate, but should note experimental status in README.

## 7. Guardrails (README.md lines 202-206)

**Documented:**
```bash
zavora-cli --guardrail-input-mode block --guardrail-output-mode redact ask "Summarize this"
```

**Reality:**
- ✅ Guardrail modes implemented: `disabled`, `observe`, `block`, `redact`
- ✅ CLI flags exist: `--guardrail-input-mode`, `--guardrail-output-mode`
- ✅ `GuardrailMode` enum in `src/cli.rs` lines 47-52
- ✅ Implementation in `src/guardrail.rs`

**Status:** Accurate.

## 8. Retrieval (README.md lines 208-212)

**Documented:**
```bash
zavora-cli --retrieval-backend local --retrieval-doc-path ./docs/knowledge.md ask "What are our standards?"
```

**Reality:**
- ✅ Retrieval backends: `disabled`, `local`, `semantic`
- ✅ CLI flags exist: `--retrieval-backend`, `--retrieval-doc-path`
- ✅ `RetrievalBackend` enum in `src/cli.rs` lines 33-37
- ✅ Implementation in `src/retrieval.rs`

**Status:** Accurate.

## Summary

### Critical Issues (Must Fix)

1. **Hook Configuration** - Documentation shows completely wrong structure and location
   - Documented: `profiles.<name>.hooks.pre_tool` with fields like `name`, `match_tool`, `match_args`, `action`, `message`
   - Actual: `agents.<name>.hooks` with fields `command`, `timeout_ms`, `max_output`, `matcher`

### Fixed Issues ✅

2. **Compaction Configuration** - IMPLEMENTED token-based auto-compaction
   - Added `compaction_threshold` and `compaction_target` fields
   - Auto-compaction now works as documented
   - New `/autocompact` command added

### Minor Issues (Should Fix)

2. **Delegate Command** - Should note experimental status in README

### Accurate Documentation

- Multi-agent orchestration with UI indicators ✅
- Built-in tools table ✅
- Agent catalogs ✅
- MCP integration ✅
- Guardrails ✅
- Retrieval ✅

## Recommendations

1. **Immediate:** Fix the hook configuration documentation in README.md (lines 177-189)
2. **Clarify:** Add "(experimental)" note to `/delegate` command documentation
3. **Document:** Add `/autocompact` command to chat commands table
4. **Consider:** Add example agent catalog file to docs showing actual hook configuration format
