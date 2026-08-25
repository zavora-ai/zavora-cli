# Subagents

Zavora treats a subagent as a governed runtime identity, not only a prompt preset. Each agent can have its own instruction, model route, tool allow/deny policy, skill allow/deny policy, turn budget, timeout, resources, and retained session.

## Portable Markdown definitions

Project definitions are discovered from these roots, in increasing precedence:

1. `.opencode/agents/`
2. `.gemini/agents/`
3. `.grok/agents/`
4. `.claude/agents/`
5. `.zavora/agents/`
6. `.agents/agents/`

Equivalent user-level roots are also discovered. Existing `.zavora/agents.toml` catalogs remain supported and have the highest precedence at their scope.

```markdown
---
name: architecture-reviewer
description: Reviews system boundaries and operational risk
provider: openai
model: gpt-5.6-sol
tools: [Read, Grep]
disallowedTools: [Bash, Write, Edit]
skills: [repository-*, source-research]
disallowedSkills: [deployment-*]
subagents: [reviewer-*, research_agent]
disallowedAgents: [reviewer-production]
permissionMode: always
maxTurns: 8
timeoutSeconds: 180
---

Review the requested design. Cite concrete repository evidence and return prioritized findings. Do not modify files.
```

Claude-style tool names such as `Read`, `Write`, `Edit`, `Bash`, `Glob`, `Grep`, and `WebFetch` are normalized to Zavora tools when imported. OpenCode-style boolean tool maps and comma-separated or YAML-list values are accepted.

## Execution

```bash
# Inspect the resolved registry and policy
zavora-cli agents list --json
zavora-cli agents show --name architecture-reviewer --json

# Direct headless run
zavora-cli agents run --name architecture-reviewer "Review this repository"

# User-directed parallel dispatch with isolated child sessions
zavora-cli agents parallel \
  --agent developer_agent \
  --agent reviewer_agent \
  "Assess the current implementation"

# Stable aggregate JSON or live JSONL completion events
zavora-cli agents parallel --output-format json \
  --agent developer_agent --agent reviewer_agent "Assess the change"

# Durable background execution with an optional isolated worktree
zavora-cli agents spawn --name developer_agent --worktree "Implement the change"
zavora-cli agents runs --json
zavora-cli agents status RUN_ID --json
zavora-cli agents send RUN_ID "Also run the integration tests"
zavora-cli agents events RUN_ID --follow
zavora-cli agents wait RUN_ID --timeout-secs 900
zavora-cli agents cancel RUN_ID
zavora-cli agents retry RUN_ID
zavora-cli agents worktree-remove RUN_ID --force
```

In classic chat and the TUI:

```text
/delegate @reviewer_agent review the current change
/delegate --agent developer_agent fix the failing test
/parallel @developer_agent @reviewer_agent assess the current implementation
/parallel --agent research_agent --agent reviewer_agent --max-concurrency 2 compare the evidence
/spawn @developer_agent --worktree implement the change
/runs
/send RUN_ID also run the integration tests
/cancel RUN_ID
/agents
```

The TUI shows foreground and durable children as independently correlated activities. Durable run state is refreshed from SQLite every 750 ms and remains visible after restarting Zavora. Foreground `Esc` cancellation and durable `/cancel RUN_ID` are distinct controls. Delegated runs emit correlated telemetry, while the durable supervisor also records an ordered, cursor-based event stream and per-run JSONL worker log.

The coordinator receives governed `spawn_agent`, `list_agent_runs`, `agent_run_status`, `send_agent_message`, `wait_agent`, `cancel_agent`, and `agent_run_events` tools. A child can therefore launch bounded grandchildren, queue follow-ups while another turn is active, wait for results, and recover completed conversations without blocking the interactive parent.

## Isolation and delegation depth

- A child receives a reduction of the parent's already-sealed tool surface. It cannot add tools or bypass profile policy, confirmation wrappers, hooks, or provenance classification.
- Tool and skill deny rules win over allow rules.
- Child-agent deny rules win over `subagents`/`agents` allow patterns. The supervisor also enforces `ZAVORA_AGENT_MAX_DEPTH` (default 2) and `ZAVORA_AGENT_MAX_ACTIVE` (default 4 per parent).
- “Trust for session” approvals are scoped to the requesting agent identity; trusting a tool for one child does not authorize its peers. Explicit global `/allow` and `--always-approve` remain process-wide by design.
- Configured agents called by the default coordinator run through ADK-Rust `AgentTool` with isolated invocation contexts.
- Named agents receive the durable supervisor tools, but recursion is bounded by the persisted parent graph, child policy, active-child limits, normal tool confirmation, and depth limits.
- Built-in specialists have category-routed tools, scoped skills, a 12-turn budget, and a bounded timeout.
- Consequential actions still require the applicable runtime approval; delegation is not authorization.
- `--worktree` creates a detached git worktree under `~/.zavora/worktrees/`; it persists for inspection and is removed only by the explicit destructive `worktree-remove --force` command.
- Cancellation verifies that the recorded PID still belongs to the exact `agents worker --run-id ...` process before signaling it, preventing a stale PID from terminating an unrelated process.

## Remaining depth

Token-level interleaved child output is retained in each run's JSONL log rather than multiplexed into the parent transcript. The activity pane is a live persistent run list, but it does not yet render an expandable tree. Remote delegation should use A2A v1.0 Agent Cards and task operations; Zavora's existing ping endpoint is only a legacy smoke contract and is not represented here as full remote-subagent support.
