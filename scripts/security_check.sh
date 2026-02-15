#!/usr/bin/env bash
set -euo pipefail

echo "[SECURITY] Checking dependency posture with cargo-audit..."
if ! cargo audit --version >/dev/null 2>&1; then
  cat >&2 <<'MSG'
[SECURITY] cargo-audit is not installed.
Install with:
  cargo install cargo-audit --locked
MSG
  exit 1
fi

cargo audit --deny warnings

echo "[SECURITY] Scanning for high-risk leaked secret patterns..."
SECRET_PATTERN='(AKIA[0-9A-Z]{16}|ASIA[0-9A-Z]{16}|-----BEGIN [A-Z ]*PRIVATE KEY-----|ghp_[A-Za-z0-9]{36}|xox[baprs]-[A-Za-z0-9-]{10,}|AIza[0-9A-Za-z_-]{35})'

if rg --hidden --glob '!.git' --glob '!target' --glob '!.zavora' --glob '!evals/reports/*' -n -S -e "${SECRET_PATTERN}" .; then
  cat >&2 <<'MSG'
[SECURITY] Potential secret material detected.
Action:
1. Remove leaked credential material from tracked files.
2. Rotate impacted credentials immediately.
3. Add preventive guardrails and re-run ./scripts/security_check.sh.
MSG
  exit 1
fi

echo "[SECURITY] Verifying .env.example does not contain real API key values..."
if rg -n '^(GOOGLE_API_KEY|OPENAI_API_KEY|ANTHROPIC_API_KEY|DEEPSEEK_API_KEY|GROQ_API_KEY)=.+$' .env.example; then
  cat >&2 <<'MSG'
[SECURITY] .env.example contains non-empty provider key values.
Action:
1. Replace with empty placeholders only.
2. Re-run ./scripts/security_check.sh.
MSG
  exit 1
fi

echo "[SECURITY] Passed."
