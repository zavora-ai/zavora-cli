# Graph Workflows and Templates

`zavora-cli` provides a graph orchestration path through:

```bash
zavora-cli workflow graph "<prompt>"
```

This mode uses ADK-Rust `GraphAgent` with conditional routing and reusable workflow templates.

## Routing Model

Execution pipeline:
1. `classify` node selects route from user input.
2. Route-specific `prepare_*` node builds a template-driven prompt.
3. `draft_response` node invokes the configured model and writes final output.

Routes:
- `release`
- `architecture`
- `risk`
- `delivery` (default fallback)

## Reusable Templates

Each route maps to a reusable template:
- `release`: objectives, release slices, acceptance criteria, rollout steps
- `architecture`: constraints, components, flow, risks
- `risk`: top risks, impact, mitigation, detection, fallback
- `delivery`: scope, implementation steps, validation, next actions

Templates are centralized in `src/main.rs` (`workflow_template`) and reused by graph branch nodes.

## Testing Coverage

Branch/routing tests include:
- deterministic route classification by intent keywords
- template availability checks for all graph routes
- graph workflow mode included in deterministic workflow test matrix

