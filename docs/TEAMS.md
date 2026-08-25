# Governed agent teams

Zavora's team surface combines ADK-Rust 2.1's portable semantic execution plane with Zavora's multi-provider agents, tools, skills, MCP connections, sessions, approvals, telemetry, and retained TUI.

Three teams work without a manifest:

- `frontier-delivery`: a governed supervisor team that delegates to research, artifact, operations, and independent-review specialists.
- `evidence-review`: parallel research and development analysis followed by an independent reviewer.
- `work-council`: a bounded shared-transcript council for artifacts, operations, and review.

```bash
zavora-cli teams list
zavora-cli teams show frontier-delivery
zavora-cli teams topology frontier-delivery --json
zavora-cli teams validate
zavora-cli teams run frontier-delivery "Implement and independently verify the change"
zavora-cli --output-format stream-json teams run evidence-review "Assess this repository"
```

The same surface is available in classic chat and the TUI through `/teams`. `/team` is an alias:

```text
/teams list
/teams show frontier-delivery
/teams run frontier-delivery implement and verify the change
```

## Portable definitions

Workspace definitions live in `.agents/teams/` or `.zavora/teams/`. User definitions live in `~/.config/zavora/teams/`. Workspace definitions override user definitions and built-ins by team name. YAML, JSON, and TOML are supported. `zavora-cli teams schema` prints the complete JSON Schema.

This example uses ADK's `TeamSpec` directly; the fields under `spec` are not a Zavora reimplementation:

```yaml
apiVersion: zavora.ai/v1alpha1
kind: team
spec:
  name: delivery
  description: Governed implementation and review
  coordinator: developer_agent
  members:
    - name: developer_agent
    - name: reviewer_agent
  relationships:
    - from: developer_agent
      to: reviewer_agent
      kind: delegate
      policy:
        timeoutMs: 120000
        approval: required
        failure:
          strategy: retry
          maxAttempts: 2
          backoffMs: 500
  policy:
    maxTransferDepth: 4
    maxDelegationDepth: 2
    maxConcurrentDelegations: 2
    context: shared
    failure: propagate
    budget:
      maxModelRequests: 20
      maxToolCalls: 60
      maxDelegations: 8
      maxWallTimeMs: 600000
```

`kind: architecture` accepts ADK's `supervisor`, `router`, and `hierarchical` templates. `kind: workflow` accepts `sequential`, `parallel`, `fanOutFanIn`, and `reviewLoop`. `kind: blackboard` accepts round-robin or allowlisted selector scheduling with bounded rounds and history.

## What ADK enforces

Before execution, ADK validates exact delegate/handoff edges, reachability, cycles, depth and concurrency bounds, relationship schemas, state/history/artifact scope, approval gates, retry/fallback/circuit-breaker policy, and aggregate event/model/tool/token/cost/time budgets.

For governed teams, Zavora registers local, plugin, and compatible imported agents through ADK's `TeamAgentRegistry`. Resolution considers exact names, capabilities, priority, version/digest constraints, health, expiry, trust labels, and authorization. ADK freezes the chosen roster in its execution receipt. Zavora records the compiled roster/routes and completed semantic receipts through the same telemetry sink used by other surfaces.

Configured is not connected: a team member only receives tools already present in the sealed runtime tool surface. Agent tool/skill allow and deny policy is applied before ADK binds the member.

## Local ADK-Rust dependency

The team APIs are from ADK-Rust 2.1, which is not yet available on crates.io in this workspace's audited environment. Install the sibling-repository override before building:

```bash
make local-adk
cargo check
```

The ignored `.cargo/config.toml` then patches every ADK crate to `../adk-rust`. `make unlink-adk` removes the override; builds will remain unavailable until compatible ADK-Rust 2.1 crates are published or another audited source is configured.
