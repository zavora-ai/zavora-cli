# Requirements Document

## Introduction

This feature adds an interactive onboarding wizard to zavora-cli that guides first-time users through selecting an AI provider, choosing a model, and configuring their API key. The wizard triggers automatically on first run (when the `.zavora` directory does not exist) and replaces the current static welcome message with a step-by-step setup flow. The result is a fully configured default profile persisted to `.zavora/config.toml`, so users can start chatting immediately without manually exporting environment variables.

## Glossary

- **Onboarding_Wizard**: The interactive terminal-based setup flow that guides users through provider, model, and API key configuration on first run.
- **Provider**: One of the supported AI backends: OpenAI, Anthropic, Google Gemini, DeepSeek, Groq, or Ollama.
- **Model**: A specific AI model offered by a Provider (e.g., "gpt-5-mini" for OpenAI, "claude-sonnet-4-20250514" for Anthropic).
- **API_Key**: A secret credential string required to authenticate with a cloud-based Provider.
- **Default_Profile**: The "default" entry in `.zavora/config.toml` that stores the user's provider, model, and other settings.
- **Config_File**: The TOML file at `.zavora/config.toml` that persists profile configurations.
- **Provider_List**: The numbered menu of available providers displayed during onboarding.
- **Model_Picker**: The numbered menu of available models for a selected provider.

## Requirements

### Requirement 1: Trigger Onboarding on First Run

**User Story:** As a new user, I want the CLI to detect that I have not configured it before, so that I am automatically guided through setup.

#### Acceptance Criteria

1. WHEN the `.zavora` directory does not exist, THE Onboarding_Wizard SHALL launch automatically before any chat session begins.
2. WHEN the `.zavora` directory already exists, THE Onboarding_Wizard SHALL not launch and the CLI SHALL proceed with normal startup.
3. WHEN the Onboarding_Wizard completes, THE CLI SHALL create the `.zavora` directory and Config_File before proceeding to the chat session.

### Requirement 2: Provider Selection

**User Story:** As a new user, I want to choose my preferred AI provider from a list, so that I can use the service I already have an account with.

#### Acceptance Criteria

1. THE Onboarding_Wizard SHALL display a Provider_List containing all supported providers: OpenAI, Anthropic, Google Gemini, DeepSeek, Groq, and Ollama.
2. WHEN displaying the Provider_List, THE Onboarding_Wizard SHALL show each provider with a numeric index for selection.
3. WHEN a user enters a valid numeric index, THE Onboarding_Wizard SHALL select the corresponding provider and proceed to model selection.
4. WHEN a user enters an invalid selection, THE Onboarding_Wizard SHALL display an error message and re-prompt the user with the Provider_List.
5. WHEN a user selects Ollama, THE Onboarding_Wizard SHALL skip the API key step and proceed to host configuration instead.

### Requirement 3: Model Selection

**User Story:** As a new user, I want to pick a model or accept the default for my chosen provider, so that I can start chatting with a model that fits my needs.

#### Acceptance Criteria

1. WHEN a provider is selected, THE Model_Picker SHALL display the available models for that provider with numeric indices, context window sizes, and short descriptions.
2. THE Model_Picker SHALL visually indicate which model is the default for the selected provider.
3. WHEN a user presses Enter without typing a selection, THE Model_Picker SHALL select the default model for the chosen provider.
4. WHEN a user enters a valid numeric index, THE Model_Picker SHALL select the corresponding model.
5. WHEN a user enters an invalid selection, THE Model_Picker SHALL display an error message and re-prompt the user with the model list.

### Requirement 4: API Key Input

**User Story:** As a new user, I want to enter my API key during setup, so that I do not have to manually configure environment variables.

#### Acceptance Criteria

1. WHEN a cloud-based provider is selected, THE Onboarding_Wizard SHALL prompt the user to enter an API key for that provider.
2. WHILE the user is typing an API key, THE Onboarding_Wizard SHALL mask the input so the key is not visible on screen.
3. WHEN a user submits an empty API key, THE Onboarding_Wizard SHALL display an error message and re-prompt for the key.
4. WHEN a user submits a non-empty API key, THE Onboarding_Wizard SHALL accept the key and proceed to the next step.

### Requirement 5: Ollama Host Configuration

**User Story:** As a user who runs models locally with Ollama, I want to configure the Ollama host URL during setup, so that the CLI connects to my local instance.

#### Acceptance Criteria

1. WHEN the user selects Ollama as the provider, THE Onboarding_Wizard SHALL prompt for the Ollama host URL.
2. THE Onboarding_Wizard SHALL display `http://localhost:11434` as the default Ollama host value.
3. WHEN the user presses Enter without typing a value, THE Onboarding_Wizard SHALL use the default host URL.
4. WHEN the user enters a custom host URL, THE Onboarding_Wizard SHALL accept and store that value.

### Requirement 6: Persist Configuration to Default Profile

**User Story:** As a new user, I want my onboarding choices saved to a config file, so that I do not have to repeat setup every time I launch the CLI.

#### Acceptance Criteria

1. WHEN the Onboarding_Wizard completes, THE CLI SHALL write the selected provider, model, and API key (or Ollama host) to the Default_Profile in the Config_File.
2. THE Config_File SHALL be written in valid TOML format consistent with the existing profile schema.
3. WHEN the Config_File is written, THE CLI SHALL be able to load the Default_Profile and resolve the provider and model without requiring environment variables.
4. IF writing the Config_File fails, THEN THE Onboarding_Wizard SHALL display a descriptive error message including the file path and the underlying OS error.

### Requirement 7: Onboarding Summary and Confirmation

**User Story:** As a new user, I want to see a summary of my choices before they are saved, so that I can verify everything is correct.

#### Acceptance Criteria

1. WHEN all onboarding steps are complete, THE Onboarding_Wizard SHALL display a summary showing the selected provider, model, and a masked representation of the API key (or Ollama host URL).
2. WHEN the user confirms the summary, THE Onboarding_Wizard SHALL persist the configuration and proceed to the chat session.
3. WHEN the user rejects the summary, THE Onboarding_Wizard SHALL restart the onboarding flow from the provider selection step.

### Requirement 8: Skip Onboarding

**User Story:** As an advanced user, I want to skip the onboarding wizard, so that I can configure the CLI manually using environment variables or config files.

#### Acceptance Criteria

1. THE Onboarding_Wizard SHALL display an option to skip setup at the provider selection step.
2. WHEN the user chooses to skip, THE Onboarding_Wizard SHALL create the `.zavora` directory with a minimal Config_File and display instructions for manual configuration.
3. WHEN the user skips onboarding, THE CLI SHALL proceed to the chat session using auto-detection from environment variables.

### Requirement 9: Re-run Onboarding via Command

**User Story:** As a user, I want to re-run the onboarding wizard at any time, so that I can change my provider or API key without manually editing config files.

#### Acceptance Criteria

1. THE CLI SHALL provide a `setup` subcommand that launches the Onboarding_Wizard regardless of whether the `.zavora` directory exists.
2. WHEN the `setup` command is run with an existing Config_File, THE Onboarding_Wizard SHALL pre-populate selections with the current profile values.
3. WHEN the `setup` command completes, THE CLI SHALL update the Default_Profile in the Config_File with the new selections.
