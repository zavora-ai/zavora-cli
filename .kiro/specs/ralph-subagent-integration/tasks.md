# Implementation Plan: Ralph Sub-Agent Integration

## Overview

Integrate adk-ralph as a path dependency into zavora-cli, adding a `zavora ralph <prompt>` CLI command that invokes Ralph's three-phase pipeline using zavora's provider configuration and telemetry infrastructure.

## Tasks

- [x] 1. Update adk-ralph to adk-rust 0.3.0 and add as dependency
  - [x] 1.1 Update adk-ralph's `Cargo.toml` to use `adk-rust = { version = "0.3.0", features = [...] }` matching zavora-cli's feature set
    - Update features list to match 0.3.2 available features (openai, anthropic, agents, models, tools, runner, sessions, telemetry → check which are still valid in 0.3.2)
    - Fix any compilation errors in adk-ralph caused by the adk-rust 0.3.2 API changes
    - Run `cargo check` in adk-ralph to confirm it compiles
    - _Requirements: 1.2, 1.3_
  - [x] 1.2 Add `adk-ralph = { path = "../adk-ralph" }` to zavora-cli's `Cargo.toml` dependencies
    - Run `cargo check` in zavora-cli to confirm dependency resolution succeeds
    - _Requirements: 1.1, 1.2_

- [x] 2. Extend CLI with Ralph command
  - [x] 2.1 Add `RalphPhase` enum and `Commands::Ralph` variant to `src/cli.rs`
    - Add `RalphPhase` enum with variants `Prd`, `Architect`, `Loop` deriving `ValueEnum`
    - Add `Commands::Ralph { prompt, phase, resume, output_dir }` with clap attributes
    - Update `command_label()` to handle `Commands::Ralph`
    - _Requirements: 2.1, 2.2, 2.3, 2.4, 2.5_

  - [ ]* 2.2 Write property test for CLI parsing (Property 1)
    - **Property 1: CLI parsing accepts any non-empty prompt**
    - **Validates: Requirements 2.1**

  - [ ]* 2.3 Write unit tests for CLI parsing edge cases
    - Test `--phase` flag with each valid value
    - Test `--resume` flag without prompt succeeds
    - Test missing prompt without `--resume` fails
    - Test `--output-dir` flag is captured
    - _Requirements: 2.2, 2.3, 2.4, 2.5_

- [x] 3. Implement configuration bridge
  - [x] 3.1 Create `src/ralph.rs` module with `RalphConfigBridge`
    - Implement `RalphConfigBridge::from_runtime_config()` that maps `RuntimeConfig` fields to `RalphConfig`
    - Implement `map_provider()` to convert zavora `Provider` enum to Ralph's provider string
    - Implement `resolve_api_key()` with profile-first, env-var-fallback logic
    - Implement `map_ralph_phase()` to convert `RalphPhase` to Ralph's internal phase type
    - Add `pub mod ralph;` to `src/lib.rs`
    - _Requirements: 3.1, 3.2, 3.3, 3.4, 3.5_

  - [ ]* 3.2 Write property test for config bridge (Property 2)
    - **Property 2: Config bridge preserves provider settings**
    - **Validates: Requirements 3.1, 3.2, 3.3, 3.5**

  - [ ]* 3.3 Write property test for unsupported provider rejection (Property 3)
    - **Property 3: Config bridge rejects unsupported providers**
    - **Validates: Requirements 3.4**

- [x] 4. Checkpoint - Ensure all tests pass
  - Ensure all tests pass, ask the user if questions arise.

- [x] 5. Implement pipeline runner and telemetry
  - [x] 5.1 Implement `run_ralph()` async function in `src/ralph.rs`
    - Create the `RalphOrchestrator` from bridged config
    - Handle `run()`, `resume()`, and `skip_to_phase()` dispatch based on CLI flags
    - Emit `ralph.started`, `ralph.completed`, and `ralph.failed` telemetry events
    - Print phase progress and output to terminal
    - _Requirements: 4.1, 4.2, 4.3, 4.4, 4.5, 4.6, 6.1, 6.2, 6.3_

  - [ ]* 5.2 Write property test for error propagation (Property 4)
    - **Property 4: Pipeline errors propagate to caller**
    - **Validates: Requirements 4.4**

  - [ ]* 5.3 Write property test for telemetry event fields (Property 5)
    - **Property 5: Telemetry start event contains required fields**
    - **Validates: Requirements 6.1**

  - [ ]* 5.4 Write unit tests for telemetry events
    - Test ralph.completed event on success
    - Test ralph.failed event on error
    - _Requirements: 6.2, 6.3_

- [x] 6. Register Ralph in agent catalog and wire into main
  - [x] 6.1 Add "ralph" entry to `implicit_agent_map()` in `src/config.rs`
    - Add ResolvedAgent with name "ralph", source Implicit, and descriptive metadata
    - _Requirements: 7.1, 7.2_

  - [x] 6.2 Add `Commands::Ralph` match arm in `run_cli()` in `src/main.rs`
    - Call `run_ralph()` with parsed arguments
    - Follow the same pattern as `Commands::Ask` for prompt enforcement and telemetry
    - _Requirements: 2.1, 4.1_

  - [ ]* 6.3 Write unit tests for agent catalog entry
    - Verify implicit_agent_map() contains "ralph" with source Implicit
    - Verify description is non-empty
    - _Requirements: 7.1, 7.2_

- [x] 7. Final checkpoint - Ensure all tests pass
  - Ensure all tests pass, ask the user if questions arise.

## Notes

- Tasks marked with `*` are optional and can be skipped for faster MVP
- adk-ralph must be updated to adk-rust 0.3.0 in task 1.1 before the path dependency can work; the adk-ralph project is at `../adk-ralph` relative to zavora-cli
- The path dependency in Cargo.toml uses `path = "../adk-ralph"` since both projects are workspace siblings
- Property tests use the `proptest` crate with minimum 100 iterations
