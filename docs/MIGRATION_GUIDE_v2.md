# Migrating Zavora CLI from v1 to v2

Zavora CLI v2 uses ADK-Rust 2.0 throughout the application. The visible configuration change is the split between the model that performs everyday work and the model that plans complex work.

## Toolchain

Rust 1.95 or newer is required; `rust-toolchain.toml` pins it.

```bash
rustup update stable
cargo install zavora-cli --version 2.0.0
```

## Model configuration

The old profile remains valid:

```toml
[profiles.default]
provider = "openai"
model = "gpt-4.1-2025-04-14"
```

`provider` and `model` now act as worker aliases. To control both roles, use:

```toml
[profiles.default]
worker_provider = "openai"
worker_model = "gpt-5.4-mini-2026-03-17"
planner_provider = "openai"
planner_model = "gpt-5.6-sol"
planner_call_budget = 4
```

The corresponding environment variables are:

```text
ZAVORA_WORKER_PROVIDER
ZAVORA_WORKER_MODEL
ZAVORA_PLANNER_PROVIDER
ZAVORA_PLANNER_MODEL
ZAVORA_PLANNER_CALL_BUDGET
```

The v1 `ZAVORA_PROVIDER` and `ZAVORA_MODEL` variables still select the worker.

## Credentials

Existing plaintext `api_key` configuration is read for compatibility, but new setup runs never write it. Run `zavora-cli setup` once to place the key in the operating-system credential vault, then remove `api_key` from project configuration.

Environment variables remain supported and take precedence over the vault.

## OpenAI default

A profile without an explicit provider now resolves to OpenAI. The default worker uses the larger shared token pool; the strong planner is called only when the worker decides that planning materially helps. Inspect the resolved choice with:

```bash
zavora-cli models
zavora-cli profiles show
```

To keep an existing provider for both roles, set both role fields explicitly or rerun setup and select that provider.

## Ralph and automation

The old `adk-ralph` 0.5 runtime has been removed. `zavora-cli ralph` now uses the same ADK-Rust 2.0 runner, tools, sessions, worker, and bounded planner as interactive chat. Review automation that relied on internal Ralph phase behavior; the public CLI flags remain available.

Zavora no longer tells an agent to run `git add -A`, commit, or push as a default completion step. Ask for those operations explicitly when required.

## Verification

These commands do not call a paid model:

```bash
zavora-cli --help
zavora-cli models
zavora-cli doctor
```

For a source checkout:

```bash
cargo fmt --all -- --check
cargo check --all-targets
cargo test --all-targets -- --test-threads=1
cargo clippy --all-targets -- -D warnings
```
