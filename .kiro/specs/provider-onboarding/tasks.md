# Implementation Plan: Provider Onboarding

## Overview

Implement an interactive onboarding wizard for zavora-cli that guides first-time users through provider selection, model selection, and API key configuration. The implementation extends the existing config and provider systems, adds a new `onboarding.rs` module, and wires it into the first-run path and a new `setup` CLI subcommand.

## Tasks

- [x] 1. Extend ProfileConfig with credential fields
  - [x] 1.1 Add `api_key: Option<String>` and `ollama_host: Option<String>` fields to `ProfileConfig` in `src/config.rs`
    - Add serde attributes consistent with existing optional fields
    - _Requirements: 6.1, 6.2_
  - [x] 1.2 Update `resolve_model()` in `src/provider.rs` to check profile `api_key` before falling back to environment variables
    - For each cloud provider branch, try `cfg.api_key` first, then `std::env::var`
    - For Ollama, try `cfg.ollama_host` first, then `OLLAMA_HOST` env var
    - _Requirements: 6.3_

- [x] 2. Create onboarding module with core logic
  - [x] 2.1 Create `src/onboarding.rs` with `OnboardingResult` struct and input parsing functions
    - Implement `parse_provider_selection(input: &str) -> Result<Option<Provider>>` (returns None for skip)
    - Implement `parse_model_selection(input: &str, options: &[ModelPickerOption]) -> Result<Option<usize>>` (returns None for default)
    - Implement `validate_api_key(input: &str) -> bool`
    - Implement `mask_api_key(key: &str) -> String`
    - Implement `format_summary(result: &OnboardingResult) -> String`
    - _Requirements: 2.3, 2.4, 3.4, 3.5, 4.3, 4.4, 7.1_
  - [ ]* 2.2 Write property test: provider selection input parsing
    - **Property 1: Provider selection input parsing**
    - **Validates: Requirements 2.3, 2.4**
  - [ ]* 2.3 Write property test: model selection input parsing
    - **Property 2: Model selection input parsing**
    - **Validates: Requirements 3.3, 3.4, 3.5**
  - [ ]* 2.4 Write property test: model catalog completeness
    - **Property 3: Model catalog completeness**
    - **Validates: Requirements 3.1**
  - [ ]* 2.5 Write property test: API key validation
    - **Property 4: API key validation**
    - **Validates: Requirements 4.3, 4.4**
  - [ ]* 2.6 Write property test: summary formatting completeness
    - **Property 6: Summary formatting completeness**
    - **Validates: Requirements 7.1**

- [x] 3. Implement interactive prompt functions
  - [x] 3.1 Implement `prompt_provider_selection()` with terminal I/O in `src/onboarding.rs`
    - Display numbered provider list with skip option
    - Read line from stdin, parse with `parse_provider_selection()`
    - Loop on invalid input with error message
    - Support pre-populated default from existing profile
    - _Requirements: 2.1, 2.2, 2.3, 2.4, 2.5_
  - [x] 3.2 Implement `prompt_model_selection()` with terminal I/O
    - Use `model_picker_options()` from `chat.rs` to get model list
    - Display models with index, context window, description, and default marker
    - Read line, parse with `parse_model_selection()`
    - Empty input returns provider default model
    - Loop on invalid input
    - _Requirements: 3.1, 3.2, 3.3, 3.4, 3.5_
  - [x] 3.3 Implement `prompt_api_key()` with masked input
    - Use crossterm raw mode to read characters individually
    - Print `*` for each character, handle backspace
    - Validate non-empty on Enter
    - _Requirements: 4.1, 4.2, 4.3, 4.4_
  - [x] 3.4 Implement `prompt_ollama_host()` with default value
    - Display default `http://localhost:11434`
    - Empty input uses default
    - _Requirements: 5.1, 5.2, 5.3, 5.4_

- [x] 4. Implement wizard orchestration and config persistence
  - [x] 4.1 Implement `run_onboarding_wizard()` that orchestrates the full flow
    - Call prompt functions in sequence: provider → (api_key | ollama_host) → model → summary
    - Handle skip path: return OnboardingResult with skipped=true
    - Handle rejection at summary: loop back to provider selection
    - Accept optional existing ProfileConfig for pre-population
    - _Requirements: 1.1, 7.2, 7.3, 8.1_
  - [x] 4.2 Implement `persist_onboarding_config()` to write config to TOML
    - Create `.zavora` directory if missing
    - Load existing ProfilesFile to preserve other profiles
    - Update default profile with OnboardingResult values
    - For skip: write minimal config with provider=Auto
    - Handle write errors with descriptive messages including path and OS error
    - _Requirements: 1.3, 6.1, 6.2, 6.4, 8.2_
  - [ ]* 4.3 Write property test: configuration round-trip
    - **Property 5: Configuration round-trip**
    - **Validates: Requirements 6.1, 6.2, 6.3, 9.3**
  - [ ]* 4.4 Write unit tests for persist and skip flows
    - Test skip produces minimal config with Auto provider
    - Test write failure to invalid path returns descriptive error
    - _Requirements: 6.4, 8.2, 8.3_

- [x] 5. Checkpoint
  - Ensure all tests pass, ask the user if questions arise.

- [x] 6. Wire into CLI and first-run path
  - [x] 6.1 Add `Setup` variant to `Commands` enum in `src/cli.rs`
    - Add `#[command(about = "Run the interactive provider setup wizard")]`
    - _Requirements: 9.1_
  - [x] 6.2 Handle `Setup` command in main dispatch
    - Load existing profile, pass to `run_onboarding_wizard(Some(&profile))`
    - Persist result and print success message
    - _Requirements: 9.1, 9.2, 9.3_
  - [x] 6.3 Update first-run path in `run_chat()` in `src/chat.rs`
    - Replace `print_onboarding()` call with `run_onboarding_wizard(None)` and `persist_onboarding_config()`
    - Reload RuntimeConfig after persisting so the chat session uses the new provider/model
    - _Requirements: 1.1, 1.2, 1.3_
  - [x] 6.4 Register `onboarding` module in `src/main.rs` (or `lib.rs`)
    - Add `mod onboarding;`
    - _Requirements: N/A (wiring)_

- [x] 7. Final checkpoint
  - Ensure all tests pass, ask the user if questions arise.

## Notes

- Tasks marked with `*` are optional and can be skipped for faster MVP
- The implementation reuses existing `Provider` enum, `model_picker_options()`, `ProfileConfig`, and `ProfilesFile` — no new dependencies beyond `proptest` (dev) and `tempfile` (dev)
- `crossterm` is already in the dependency tree for terminal interactions
- Property tests use `proptest` crate with minimum 100 iterations each
