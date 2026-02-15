# Server Mode and A2A Runtime

`zavora-cli` supports a server mode for health checks, prompt execution, and A2A handshake smoke flows.

## Startup

```bash
cargo run -- server serve --host 127.0.0.1 --port 8787
```

- Default bind: `127.0.0.1:8787`
- Active profile/runtime config still applies (provider, model, retrieval, guardrails, sessions).

## Endpoints

### `GET /healthz`

Returns process/app health for startup/readiness checks.

### `POST /v1/ask`

Request:

```json
{
  "prompt": "Draft release risks",
  "session_id": "optional-session",
  "user_id": "optional-user"
}
```

Response:

```json
{
  "answer": "...",
  "provider": "openai",
  "model": "gpt-4o-mini",
  "session_id": "...",
  "user_id": "..."
}
```

### `POST /v1/a2a/ping`

Request:

```json
{
  "from_agent": "sales-agent",
  "to_agent": "procurement-agent",
  "message_id": "msg-001",
  "correlation_id": "corr-001",
  "payload": { "intent": "supply-check" }
}
```

Response fields include:

- `acknowledged_message_id`
- `correlation_id`
- `status=acknowledged`

## Smoke Validation

```bash
cargo run -- server a2a-smoke
```

This validates the A2A ping contract path and confirms request/ack correlation handling.

## Coexistence Guarantee

Server mode is additive and uses the same runtime config model as CLI commands. Existing CLI command behavior remains unchanged, with shared quality gates (`make quality-gate`) covering regression checks.
