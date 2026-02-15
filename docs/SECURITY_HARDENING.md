# Security Hardening Controls

This document captures the GA hardening controls for secret handling, dependency posture, and operational practices.

## Secret Handling

- Secrets are sourced from environment variables only (never committed in config files).
- `.env.example` intentionally contains empty provider credential placeholders.
- Guardrail policies can be configured to block/redact sensitive terms at input/output boundaries.
- Telemetry events should avoid raw sensitive payload capture; use summarized outcomes where possible.

## Dependency Posture

- Security dependency audit command:

```bash
make security-check
```

`make security-check` runs:

1. `cargo audit --deny warnings`
2. high-risk leaked secret pattern scan via `scripts/security_check.sh`
3. `.env.example` non-empty key-value protection checks

Known upstream advisories are tracked in `.cargo/audit.toml` with temporary `ignore` entries.
These entries are only for transitive dependencies owned upstream; review and retire them as part of each release cycle.

## Release Workflow Integration

- Release pipeline (`.github/workflows/release.yml`) installs `cargo-audit` and runs `make security-check` before building release artifacts.
- Release checklist includes `make security-check` as a required gate.

## Operational Controls

- Use least-privilege API keys for model providers.
- Rotate keys immediately if leakage is suspected.
- Keep telemetry enabled and review `guardrail.*` and `command.failed` events.
- Prefer sqlite session storage only where persistence is required; secure file permissions in deployment environments.
- Enforce CI quality and security gates prior to release tagging.

## Incident Response Baseline

When security checks fail:

1. Stop release promotion.
2. Triage findings (dependency advisory or possible secret leak).
3. Remediate and rotate credentials if needed.
4. Re-run `make security-check` and quality gates.
5. Update release notes with remediation summary.
