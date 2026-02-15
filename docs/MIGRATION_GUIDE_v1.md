# Migration Guide: pre-1.0 to v1.0.0

This guide covers migration from pre-1.0 `zavora-cli` builds (`v0.x`) to GA `v1.0.0`.

## Migration Goals

- Preserve existing workflows (`ask`, `chat`, `workflow`, `release-plan`)
- Adopt GA safety/quality defaults
- Validate server/A2A, security, and performance readiness

## 1. Backup Existing State

Before upgrading:

```bash
cp .zavora/config.toml .zavora/config.toml.bak || true
cp .zavora/sessions.db .zavora/sessions.db.bak || true
```

If you use custom config paths, back up those files as well.

## 2. Upgrade Binary

Build or install GA release:

```bash
cargo build --release
```

Then verify:

```bash
./target/release/zavora-cli --help
```

## 3. Update Profile Config To GA Baseline

Ensure these fields exist in active profiles:

- `tool_confirmation_mode` (recommended: `mcp-only`)
- `telemetry_enabled`, `telemetry_path`
- `guardrail_input_mode`, `guardrail_output_mode`
- `guardrail_terms`, `guardrail_redact_replacement`
- `tool_timeout_secs`, `tool_retry_attempts`, `tool_retry_delay_ms`

Example:

```toml
[profiles.default]
provider = "openai"
model = "gpt-4o-mini"
session_backend = "sqlite"
session_db_url = "sqlite://.zavora/sessions.db"
tool_confirmation_mode = "mcp-only"
telemetry_enabled = true
telemetry_path = ".zavora/telemetry/events.jsonl"
guardrail_input_mode = "observe"
guardrail_output_mode = "redact"
guardrail_terms = ["password", "secret", "api key", "private key"]
guardrail_redact_replacement = "[REDACTED]"
tool_timeout_secs = 45
tool_retry_attempts = 2
tool_retry_delay_ms = 500
```

## 4. Validate Runtime Compatibility

Run migration smoke checks:

```bash
zavora-cli doctor
zavora-cli profiles show
zavora-cli server a2a-smoke
```

If using server mode:

```bash
zavora-cli server serve --host 127.0.0.1 --port 8787
curl -sS http://127.0.0.1:8787/healthz
```

## 5. Validate Release Gates

Run GA gate set:

```bash
make release-check
make perf-check
```

## 6. Known Behavior Changes In GA

- Server `/v1/ask` now reuses cached runners per `(user_id, session_id)` for lower repeated-call overhead.
- Security gate includes RustSec audit policy from `.cargo/audit.toml`; ignored transitive advisories must be reviewed each release.
- GA introduces explicit perf/reliability thresholds with report output (`.zavora/perf/latest.json`).

## 7. Rollback

If migration fails:

1. Restore `config.toml` and session DB backups.
2. Revert to previous tagged binary.
3. Run `zavora-cli doctor` and `zavora-cli server a2a-smoke`.
4. Open a migration regression issue with command output and config diff.
