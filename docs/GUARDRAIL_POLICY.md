# Guardrail Policy Framework

`zavora-cli` provides configurable input/output guardrail enforcement for runtime safety.

## Configuration Schema

Profile keys:

- `guardrail_input_mode` (`disabled|observe|block|redact`)
- `guardrail_output_mode` (`disabled|observe|block|redact`)
- `guardrail_terms` (list of case-insensitive match terms)
- `guardrail_redact_replacement` (replacement text for redaction mode)

CLI/env overrides:

- `--guardrail-input-mode` / `ZAVORA_GUARDRAIL_INPUT_MODE`
- `--guardrail-output-mode` / `ZAVORA_GUARDRAIL_OUTPUT_MODE`
- `--guardrail-term` / `ZAVORA_GUARDRAIL_TERM`
- `--guardrail-redact-replacement` / `ZAVORA_GUARDRAIL_REDACT_REPLACEMENT`

## Enforcement Behavior

- `input` policy applies before prompt execution.
- `output` policy applies before final answer is printed.
- Chat mode streams normally when output mode is `disabled`/`observe`.
- Chat mode uses buffered rendering when output mode is `block`/`redact` so enforcement happens before printing.

## Observable Outcomes

Guardrail matches emit telemetry events:

- `guardrail.input.observed`
- `guardrail.input.blocked`
- `guardrail.input.redacted`
- `guardrail.output.observed`
- `guardrail.output.blocked`
- `guardrail.output.redacted`

Each event includes direction, mode, hit count, and matched terms.

## Testing Coverage

Guardrail tests cover:

- redaction path (`redact` mode)
- blocking path (`block` mode)
- non-mutating observation path (`observe` mode)
