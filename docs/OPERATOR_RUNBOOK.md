# Operator Runbook (GA)

This runbook covers baseline operations for `zavora-cli` in GA (`v1.0.0`) environments.

## Scope

- CLI interactive/runtime usage
- Server mode operations (`server serve`)
- Session persistence and retention
- Telemetry, security, and performance gates
- Release/rollback execution flow

## Runtime Prerequisites

- Rust runtime with `zavora-cli` binary available
- Provider credentials configured in environment (`OPENAI_API_KEY`, `GOOGLE_API_KEY`, etc.)
- Persistent session store configured if cross-run history is required (`sqlite`)
- Writable directories for:
  - `.zavora/sessions.db` (when sqlite backend is used)
  - `.zavora/telemetry/events.jsonl`
  - `.zavora/evals/*` and `.zavora/perf/*` for gate outputs

## Recommended Production Profile

```toml
[profiles.default]
provider = "openai"
model = "gpt-4o-mini"
session_backend = "sqlite"
session_db_url = "sqlite://.zavora/sessions.db"
telemetry_enabled = true
telemetry_path = ".zavora/telemetry/events.jsonl"
tool_confirmation_mode = "mcp-only"
guardrail_input_mode = "observe"
guardrail_output_mode = "redact"
guardrail_terms = ["password", "secret", "api key", "private key"]
```

## Start/Stop Procedures

### CLI mode

```bash
zavora-cli chat
```

### Server mode

```bash
zavora-cli server serve --host 127.0.0.1 --port 8787
```

Health checks:

```bash
curl -sS http://127.0.0.1:8787/healthz
curl -sS -X POST http://127.0.0.1:8787/v1/a2a/ping \
  -H 'Content-Type: application/json' \
  -d '{"from_agent":"ops","to_agent":"zavora","message_id":"ping-1","payload":{}}'
```

## Session Operations

- List sessions:

```bash
zavora-cli --session-backend sqlite --session-db-url sqlite://.zavora/sessions.db sessions list
```

- Show recent events:

```bash
zavora-cli --session-backend sqlite --session-db-url sqlite://.zavora/sessions.db sessions show --session-id default-session --recent 30
```

- Retention prune (safe preview first):

```bash
zavora-cli --session-backend sqlite --session-db-url sqlite://.zavora/sessions.db sessions prune --keep 50 --dry-run
zavora-cli --session-backend sqlite --session-db-url sqlite://.zavora/sessions.db sessions prune --keep 50 --force
```

## Observability And Health

- Runtime diagnostics:

```bash
zavora-cli doctor
zavora-cli telemetry report --limit 2000
```

- Watch for:
  - elevated `command.failed`
  - sustained `guardrail.*.blocked` spikes
  - repeated tool failure events

## Security And Quality Gates

Required gates before release tag:

```bash
make release-check
make perf-check
```

Additional release checks:

```bash
make security-check
make quality-gate
```

## Incident Response Baseline

1. Stop release promotion or scale-down impacted workload.
2. Collect logs (`telemetry/events.jsonl`) and failing command inputs.
3. Run `make security-check` and `make perf-check` to classify regression type.
4. If secrets are suspected: rotate credentials immediately and audit `.env` sources.
5. If reliability/perf regression is confirmed: rollback to previous stable tag and open a follow-up issue.

## Rollback Procedure

1. Checkout previous release tag (for example `v0.3.1` or latest stable).
2. Restore backed-up `.zavora/config.toml` and `sessions.db` if schema/config diverged.
3. Re-run smoke checks:
   - `zavora-cli doctor`
   - `zavora-cli server a2a-smoke`
4. Announce rollback and attach incident summary.
