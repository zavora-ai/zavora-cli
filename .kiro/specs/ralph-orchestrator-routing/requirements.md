# Requirements Document

## Introduction

This feature adds two complementary ways to invoke the Ralph autonomous development pipeline from within the zavora interactive chat session:

1. A `/ralph <prompt>` slash command for direct, explicit invocation of Ralph from the chat REPL.
2. Autonomous routing from the Orchestrator, so that when agent mode is active the Orchestrator can classify incoming work and delegate greenfield/multi-phase development tasks to Ralph without the user having to explicitly choose.

Together these make Ralph a first-class sub-agent of the zavora chat experience rather than a standalone CLI-only tool.

## Glossary

- **Chat_REPL**: The interactive chat loop implemented in `run_chat()` (`src/chat.rs`) that reads user input, dispatches commands, and streams LLM responses.
- **Slash_Command**: A user-typed `/command <arg>` string parsed by `parse_chat_command()` and executed by `dispatch_chat_command()`.
- **Orchestrator**: The `Orchestrator` struct (`src/agents/orchestrator.rs`) that coordinates capability agents through a Bootstrap → Gather → Plan → Execute → Verify → Repair → Commit pipeline.
- **Ralph_Pipeline**: The autonomous development pipeline exposed by `run_ralph()` (`src/ralph.rs`) that runs PRD → Architect → Implementation phases.
- **Task_Classifier**: Superseded — the LLM handles classification via `transfer_to_agent` routing to the ralph_agent sub-agent.
- **ralph_agent**: A sub-agent built via `LlmAgentBuilder` that wraps the Ralph_Pipeline as a tool, registered on the main assistant when Agent_Mode is active.
- **Agent_Mode**: A chat session state toggled by `/agent` that auto-approves fs_read, fs_write, and execute_bash tools.
- **RuntimeConfig**: The `RuntimeConfig` struct (`src/config.rs`) carrying provider, model, API key, and session settings.

## Requirements

### Requirement 1: /ralph Slash Command

**User Story:** As a developer using the zavora chat, I want to type `/ralph <prompt>` to invoke the Ralph pipeline directly, so that I can launch multi-phase autonomous development without leaving the chat session.

#### Acceptance Criteria

1. WHEN a user types `/ralph <prompt>` in the Chat_REPL, THE parse_chat_command function SHALL return a `ChatCommand::Ralph(String)` variant containing the prompt text.
2. WHEN a user types `/ralph` with no prompt argument, THE dispatch_chat_command function SHALL print a usage message and return `ChatCommandAction::Continue` without invoking the Ralph_Pipeline.
3. WHEN dispatch_chat_command receives a `ChatCommand::Ralph` with a non-empty prompt, THE dispatch_chat_command function SHALL invoke `run_ralph()` with the current RuntimeConfig, the prompt, no phase override, resume set to false, and no output directory override.
4. WHEN the Ralph_Pipeline completes successfully after a `/ralph` invocation, THE dispatch_chat_command function SHALL print a completion message and return `ChatCommandAction::Continue`.
5. IF the Ralph_Pipeline returns an error after a `/ralph` invocation, THEN THE dispatch_chat_command function SHALL print the error to stderr and return `ChatCommandAction::Continue` without crashing the Chat_REPL.
6. THE print_chat_help function SHALL include `/ralph <prompt>` in the displayed command list with a description indicating it runs the Ralph autonomous development pipeline.

### Requirement 2: Ralph Sub-Agent Registration

**User Story:** As a developer, I want the system to automatically route greenfield/multi-phase development tasks to Ralph when agent mode is active, so that the right agent handles each type of work without manual intervention.

#### Acceptance Criteria

1. THE ralph_agent sub-agent SHALL be built with a description and instruction that guide the LLM to route greenfield project creation, multi-file scaffolding, and multi-phase development work to Ralph.
2. THE ralph_agent sub-agent SHALL expose a `run_ralph_pipeline` tool that accepts a prompt string and invokes the existing `run_ralph()` function.
3. WHILE Agent_Mode is active, THE runner SHALL attach the ralph_agent as a sub-agent to the main assistant via `LlmAgentBuilder::sub_agent()`.
4. WHILE Agent_Mode is not active, THE runner SHALL omit the ralph_agent from the main assistant's sub-agent list.
5. WHEN the ralph_agent sub-agent fails to build, THE runner SHALL log a warning and continue without Ralph routing capability.

### Requirement 3: Ralph Sub-Agent Instruction and Behavior

**User Story:** As a developer in agent mode, I want the LLM to autonomously decide when to delegate to Ralph based on the nature of my request, so that I get the best agent for the job without manually choosing.

#### Acceptance Criteria

1. THE ralph_agent instruction SHALL describe the types of tasks Ralph handles: greenfield project creation, multi-phase development, and large-scale scaffolding.
2. THE ralph_agent instruction SHALL describe the types of tasks Ralph does not handle: targeted edits, questions, single-file changes, and debugging.
3. WHEN the `run_ralph_pipeline` tool is invoked, THE tool SHALL call `run_ralph()` with the prompt, no phase override, resume set to false, and no output directory override.
4. IF the `run_ralph_pipeline` tool encounters an error from `run_ralph()`, THEN THE tool SHALL return the error through the adk-rust tool result mechanism.
5. THE system prompt in `src/runner.rs` SHALL list ralph_agent in the SUBAGENTS section with a note that it is enabled only in agent mode.

### Requirement 4: Telemetry for Ralph Routing

**User Story:** As a system operator, I want telemetry events emitted for Ralph routing decisions, so that I can observe how often Ralph is invoked and through which path.

#### Acceptance Criteria

1. WHEN the `/ralph` Slash_Command is dispatched, THE dispatch_chat_command function SHALL emit a `chat.ralph.invoked` telemetry event containing the provider and model.
2. WHEN the `run_ralph_pipeline` tool is invoked via the ralph_agent sub-agent, THE tool SHALL emit a `ralph_agent.tool.invoked` telemetry event containing the prompt length.

### Requirement 5: Help and Discoverability

**User Story:** As a developer new to zavora, I want the `/ralph` command to be documented alongside other commands, so that I can discover it through the help system.

#### Acceptance Criteria

1. THE print_chat_help function SHALL list `/ralph <prompt>` in the Commands section between `/orchestrate` and `/tools`.
2. WHEN a user types an unknown command that is a close misspelling of "ralph", THE parse_chat_command function SHALL return `UnknownCommand` with the misspelled text so the existing unknown-command handling applies.
