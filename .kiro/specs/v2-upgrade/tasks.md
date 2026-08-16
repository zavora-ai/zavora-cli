# Zavora CLI v2 tasks

> **Superseded by [`.kiro/specs/v2-vision`](../v2-vision/requirements.md) on 2026-08-15.**
> That spec is the authoritative v2 contract and holds the reconciled backlog.
> This document is retained for provenance. Checkbox state here was reconciled
> against the runtime on 2026-08-15; where the implementation deliberately
> diverged, the divergence is recorded in v2-vision task 23.4.

- [x] Audit the existing repository and preserve pre-existing work.
- [x] Define requirements and runtime design.
- [x] Migrate dependencies and toolchain metadata to ADK-Rust 2.0.
- [x] Add a provider-neutral planner/worker model-role configuration.
- [x] Add the quota-aware OpenAI model catalog.
- [x] Compose the bounded planner through ADK-Rust `AgentTool`.
- [x] Add role commands and provider/model switching.
- [x] Replace the startup banner and generated greeting with a compact runtime surface.
- [x] Update onboarding, README, changelog, and migration guidance.
- [x] Add deterministic tests for role routing and planner budgets.
- [x] Run formatting, compilation, tests, Clippy, and CLI smoke checks.
- [x] Commit the verified upgrade.
