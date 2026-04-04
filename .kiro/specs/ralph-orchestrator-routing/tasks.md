# Implementation Plan: Ralph Orchestrator Routing

## Overview

Add `/ralph <prompt>` as a chat slash command and register Ralph as an adk-rust sub-agent on the main assistant for autonomous LLM-based routing when agent mode is active. Builds on the existing `run_ralph()` infrastructure from the ralph-subagent-integration spec.

## Tasks

- [x] 1. Add /ralph chat slash command
  - [x] 1.1 Add `Ralph(String)` variant to `ChatCommand` enum and parse arm in `parse_chat_command()` in `src/chat.rs`
    - Add `Ralph(String)` to the `ChatCommand` enum
    - Add `"ralph" => ParsedChatCommand::Command(ChatCommand::Ralph(arg.to_string()))` match arm
    - _Requirements: 1.1_

  - [x] 1.2 Add `ChatCommand::Ralph` dispatch handler in `dispatch_chat_command()` in `src/chat.rs`
    - Empty prompt: print usage and return Continue
    - Non-empty prompt: emit `chat.ralph.invoked` telemetry, call `run_ralph(cfg, prompt, None, false, None, telemetry)`, print result or error
    - _Requirements: 1.2, 1.3, 1.4, 1.5, 4.1_

  - [x] 1.3 Add `/ralph` to `print_chat_help()` in `src/chat.rs`
    - Place after the `/orchestrate` line in the Commands section
    - _Requirements: 1.6, 5.1_

  - [ ]* 1.4 Write property test for /ralph command parsing
    - **Property 1: Parse round-trip for /ralph command**
    - **Validates: Requirements 1.1**

  - [ ]* 1.5 Write unit tests for /ralph dispatch edge cases
    - Test empty prompt shows usage
    - Test unknown command "/rlaph" returns UnknownCommand
    - _Requirements: 1.2, 5.2_

- [x] 2. Checkpoint - Verify /ralph slash command works
  - Ensure all tests pass, ask the user if questions arise.

- [x] 3. Build Ralph sub-agent module
  - [x] 3.1 Create `src/agents/ralph_agent.rs` with `RalphPipelineTool` and `build_ralph_agent()`
    - Implement `RalphPipelineTool` struct holding `Arc<RuntimeConfig>` and `Arc<TelemetrySink>`
    - Implement `Tool` trait: name `run_ralph_pipeline`, parameter schema with `prompt` string field
    - Implement `execute()` to call `run_ralph()` with the prompt, emit `ralph_agent.tool.invoked` telemetry
    - Implement `build_ralph_agent()` using `LlmAgentBuilder` with instruction describing greenfield/multi-phase routing
    - Register the module in `src/agents/mod.rs`
    - _Requirements: 2.1, 2.2, 3.1, 3.2, 3.3, 3.4, 4.2_

  - [ ]* 3.2 Write unit tests for RalphPipelineTool
    - Test tool name and description
    - Test parameter schema contains "prompt" field
    - _Requirements: 2.2, 3.3_

- [x] 4. Wire Ralph sub-agent into runner
  - [x] 4.1 Add conditional ralph sub-agent registration in `build_agent()` in `src/runner.rs`
    - Add `build_ralph_subagent_if_agent_mode()` function that checks `is_agent_mode()` and calls `build_ralph_agent()`
    - Attach via `builder.sub_agent(ralph_agent)` alongside existing search sub-agent
    - _Requirements: 2.3, 2.4, 2.5_

  - [x] 4.2 Update system prompt in `src/runner.rs` to mention ralph_agent in SUBAGENTS section
    - Add `- ralph_agent: For greenfield projects and multi-phase development (enabled only in agent mode)`
    - _Requirements: 3.5_

  - [ ]* 4.3 Write unit test for conditional sub-agent attachment
    - Verify ralph sub-agent is not attached when agent mode is off
    - _Requirements: 2.4_

- [x] 5. Final checkpoint - Ensure all tests pass
  - Ensure all tests pass, ask the user if questions arise.

## Notes

- Tasks marked with `*` are optional and can be skipped for faster MVP
- The existing `run_ralph()` in `src/ralph.rs` is reused as-is — no modifications needed
- The `RalphPipelineTool` needs `Arc<RuntimeConfig>` and `Arc<TelemetrySink>` passed at construction time
- Property tests use the `proptest` crate with minimum 100 iterations
- The ralph sub-agent follows the exact same pattern as `search_agent` in `src/agents/search.rs`
