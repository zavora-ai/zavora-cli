# MCP Toolset Manager

This document describes how `zavora-cli` registers MCP toolsets from profile configuration, discovers tools, and wires them into runtime agent execution.

## Profile Schema

MCP servers are configured per profile in `.zavora/config.toml`.

```toml
[profiles.default]
provider = "openai"
model = "gpt-4o-mini"

[[profiles.default.mcp_servers]]
name = "ops-tools"
endpoint = "https://mcp.example.com/ops"
enabled = true
timeout_secs = 15
auth_bearer_env = "OPS_MCP_TOKEN"
tool_allowlist = ["search_incidents", "get_runbook"]
```

Fields:
- `name` (required): unique logical server name within a profile.
- `endpoint` (required): MCP HTTP endpoint.
- `enabled` (optional, default `true`): whether server participates in discovery/runtime registration.
- `timeout_secs` (optional, default `15`): connection/discovery timeout.
- `auth_bearer_env` (optional): environment variable name that stores bearer token.
- `tool_allowlist` (optional): if set, only listed tool names are exposed.
- `tool_confirmation_mode` (profile-level, optional): `never`, `mcp-only` (default), or `always`.
- `require_confirm_tool` (profile-level, optional): extra tool names that require confirmation.
- `approve_tool` (profile-level, optional): required tools that are auto-approved.
- `tool_timeout_secs` (profile-level, optional): timeout used for single-agent tool execution.
- `tool_retry_attempts` (profile-level, optional): retry attempts for MCP reconnect/retry behavior.
- `tool_retry_delay_ms` (profile-level, optional): delay between MCP retries.

## Commands

List enabled servers:

```bash
zavora-cli --profile default mcp list
```

Discover tools:

```bash
zavora-cli --profile default mcp discover
zavora-cli --profile default mcp discover --server ops-tools
```

`mcp discover` behavior:
- prints tool names for each reachable server.
- returns non-zero when one or more servers fail.
- emits actionable failure details for endpoint/auth issues.

## Runtime Registration Flow

Tool registration for runtime execution:
1. Build built-in function tools (`current_unix_time`, `release_template`).
2. Discover tools from enabled MCP servers.
3. Merge both sets into one runtime toolset.
4. Attach runtime toolset to single-agent execution paths.
5. Resolve tool confirmation policy and per-tool approve/deny decisions.

Current integration points:
- `ask`
- `chat` (including `/provider` and `/model` runner rebuilds)
- `workflow single`

Tool confirmation behavior:
- default mode is `mcp-only`, so MCP tools require confirmation.
- required tools without explicit approval are denied.
- explicit approvals can be configured in profile (`approve_tool`) or CLI (`--approve-tool`).

If MCP discovery fails during runtime initialization:
- that server is skipped.
- warning is logged.
- CLI continues with available tools.

## Failure Handling

Failure cases covered:
- missing/disabled server selection
- missing bearer token environment variable
- endpoint connection failures/timeouts
- reconnect/retry exhaustion failures

Error classification:
- MCP failures are categorized as `TOOLING` by CLI taxonomy.
- hints direct operators to check tool config and rerun with `RUST_LOG=info`.
- tool lifecycle emits structured telemetry logs (`requested`, `succeeded`, `failed`).
