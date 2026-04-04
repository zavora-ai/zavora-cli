# Requirements Document

## Introduction

This document specifies the requirements for integrating the adk-ralph autonomous development system as a sub-agent into the zavora-cli project. The integration enables zavora-cli users to invoke Ralph's three-phase pipeline (PRD → Architect → Ralph Loop) through a new CLI command, while reusing zavora's provider configuration and coexisting with zavora's existing agent and tool systems.

## Glossary

- **Zavora_CLI**: The Rust CLI AI agent shell built on adk-rust 0.3.0, serving as the host application
- **Ralph_Pipeline**: The adk-ralph three-phase autonomous development pipeline consisting of PRD Agent, Architect Agent, and Ralph Loop Agent
- **Ralph_Orchestrator**: The RalphOrchestrator struct from adk-ralph that manages pipeline execution with run(), resume(), and skip_to_phase() methods
- **Ralph_Config**: The configuration structure for the Ralph pipeline, originally loaded from environment variables
- **Provider_Config**: Zavora's TOML-based profile configuration that specifies model provider, API keys, and runtime settings
- **Agent_Catalog**: Zavora's TOML-based system for registering and selecting named agent profiles
- **Runtime_Config**: The resolved configuration struct in zavora-cli that merges CLI flags, profile settings, and agent overrides

## Requirements

### Requirement 1: Dependency Integration

**User Story:** As a developer, I want adk-ralph added as a dependency to zavora-cli, so that Ralph's pipeline code is available for use within the zavora-cli codebase.

#### Acceptance Criteria

1. THE Zavora_CLI build system SHALL include adk-ralph as a path dependency pointing to the local adk-ralph project directory
2. WHEN the zavora-cli project is compiled, THE build system SHALL resolve adk-ralph and its transitive dependencies without version conflicts with adk-rust 0.3.0
3. IF adk-ralph uses an incompatible adk-rust version, THEN THE build system SHALL fail with a clear dependency resolution error

### Requirement 2: CLI Command

**User Story:** As a user, I want a `zavora ralph` CLI command, so that I can invoke the Ralph autonomous development pipeline from the zavora terminal.

#### Acceptance Criteria

1. WHEN a user runs `zavora ralph <prompt>`, THE Zavora_CLI SHALL accept the prompt and initiate the Ralph pipeline
2. THE Zavora_CLI SHALL support an optional `--phase` flag on the ralph command that accepts values "prd", "architect", or "loop" to skip directly to a specific pipeline phase
3. THE Zavora_CLI SHALL support an optional `--resume` flag on the ralph command to resume a previously interrupted pipeline run
4. WHEN a user runs `zavora ralph` without a prompt and without `--resume`, THE Zavora_CLI SHALL display a usage error indicating that a prompt is required
5. THE Zavora_CLI SHALL support an optional `--output-dir` flag on the ralph command to specify where Ralph writes its artifacts, defaulting to the current working directory

### Requirement 3: Provider Configuration Bridge

**User Story:** As a user, I want Ralph to use my existing zavora provider configuration, so that I do not need to set separate environment variables for Ralph.

#### Acceptance Criteria

1. WHEN the ralph command is invoked, THE Zavora_CLI SHALL construct a Ralph_Config from the active Provider_Config profile settings
2. WHEN the active profile specifies a provider and model, THE configuration bridge SHALL map the zavora provider name and model name to the corresponding Ralph_Config fields
3. WHEN the active profile specifies an API key, THE configuration bridge SHALL pass the API key to Ralph_Config without requiring a separate environment variable
4. IF the active profile's provider is not supported by Ralph, THEN THE Zavora_CLI SHALL return an error naming the unsupported provider and listing supported alternatives
5. WHEN CLI flags `--provider` or `--model` override the profile, THE configuration bridge SHALL use the overridden values for Ralph_Config

### Requirement 4: Pipeline Execution

**User Story:** As a user, I want the Ralph pipeline to execute its three phases and stream output to my terminal, so that I can observe the autonomous development process.

#### Acceptance Criteria

1. WHEN the ralph command is invoked with a prompt, THE Ralph_Pipeline SHALL execute the PRD, Architect, and Ralph Loop phases in sequence
2. WHILE the Ralph_Pipeline is executing, THE Zavora_CLI SHALL stream phase progress and agent output to the terminal
3. WHEN a phase completes, THE Zavora_CLI SHALL display a summary indicating the completed phase name and transition to the next phase
4. IF the Ralph_Pipeline encounters an error during execution, THEN THE Zavora_CLI SHALL display the error message and exit with a non-zero status code
5. WHEN the `--phase` flag is provided, THE Ralph_Pipeline SHALL skip directly to the specified phase using the skip_to_phase() method
6. WHEN the `--resume` flag is provided, THE Ralph_Pipeline SHALL resume from the last checkpoint using the resume() method

### Requirement 5: Tool Coexistence

**User Story:** As a developer, I want Ralph's tools to coexist with zavora's built-in tools without conflicts, so that both systems operate correctly during pipeline execution.

#### Acceptance Criteria

1. WHEN the Ralph_Pipeline executes, THE Zavora_CLI SHALL provide Ralph with its own isolated tool set from the adk-ralph crate
2. THE Zavora_CLI SHALL keep Ralph's tool registrations separate from zavora's built-in tool registrations during pipeline execution
3. IF Ralph's tools and zavora's tools share a tool name, THEN THE Zavora_CLI SHALL use Ralph's tool version within the Ralph pipeline context

### Requirement 6: Telemetry Coexistence

**User Story:** As a developer, I want Ralph's execution to emit telemetry events through zavora's telemetry system, so that pipeline runs are tracked alongside other zavora operations.

#### Acceptance Criteria

1. WHEN the ralph command starts, THE Zavora_CLI SHALL emit a telemetry event with the command name, provider, and model
2. WHEN the ralph command completes, THE Zavora_CLI SHALL emit a telemetry event with the duration and completion status
3. IF the ralph command fails, THEN THE Zavora_CLI SHALL emit a telemetry event with the error details

### Requirement 7: Agent Catalog Registration

**User Story:** As a user, I want Ralph to appear in the zavora agent catalog, so that I can discover and inspect the Ralph pipeline alongside other agents.

#### Acceptance Criteria

1. WHEN a user runs `zavora agents list`, THE Agent_Catalog SHALL include a "ralph" entry with source "implicit"
2. WHEN a user runs `zavora agents show --name ralph`, THE Agent_Catalog SHALL display Ralph's description, supported providers, and pipeline phase information
