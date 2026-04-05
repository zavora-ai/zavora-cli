use std::io::{self, BufRead, Write};
use std::path::Path;

use anyhow::{bail, Context, Result};
use crossterm::event::{read, Event, KeyCode, KeyEvent, KeyModifiers};
use crossterm::terminal::{disable_raw_mode, enable_raw_mode};

/// Create `.skills/` with a sample skill if it doesn't exist.
pub fn ensure_skills_dir() {
    let skills_dir = Path::new(".skills");
    if skills_dir.exists() {
        return;
    }
    if std::fs::create_dir_all(skills_dir).is_err() {
        return;
    }
    let sample = r#"---
name: sample-skill
description: A sample skill. Edit this file or add new .md files to .skills/ to teach Zavora new capabilities.
---

# Sample Skill

This is a placeholder skill. Replace this with instructions for your use case.

Skills are automatically discovered and injected into the agent's context
when their description matches the user's request.

## Format

- `name`: short identifier (lowercase, hyphens)
- `description`: when the agent should use this skill (trigger conditions)
- Body: markdown instructions the agent follows
"#;
    let _ = std::fs::write(skills_dir.join("sample-skill.md"), sample);
}

use crate::chat::ModelPickerOption;
use crate::cli::Provider;
use crate::config::{ProfileConfig, load_profiles};

/// Captures the user's selections from the onboarding wizard.
pub struct OnboardingResult {
    pub provider: Provider,
    pub model: String,
    pub api_key: Option<String>,
    pub ollama_host: Option<String>,
    pub skipped: bool,
}

/// Parses provider selection input.
///
/// Returns `Ok(Some(Provider))` for valid numeric input 1–6,
/// `Ok(None)` for "s"/"S" (skip), or an error for anything else.
pub fn parse_provider_selection(input: &str) -> Result<Option<Provider>> {
    match input.trim() {
        "1" => Ok(Some(Provider::Openai)),
        "2" => Ok(Some(Provider::Anthropic)),
        "3" => Ok(Some(Provider::Gemini)),
        "4" => Ok(Some(Provider::Deepseek)),
        "5" => Ok(Some(Provider::Groq)),
        "6" => Ok(Some(Provider::Ollama)),
        "s" | "S" => Ok(None),
        other => bail!(
            "Invalid selection '{}'. Please enter a number 1–6 or 's' to skip.",
            other
        ),
    }
}
/// Displays the provider selection menu and reads user input in a loop.
///
/// Returns `Ok(Some(Provider))` when a valid provider is selected,
/// or `Ok(None)` when the user chooses to skip.
/// If `default` is `Some`, the corresponding provider is marked with `[current]`.
pub fn prompt_provider_selection(default: Option<Provider>) -> Result<Option<Provider>> {
    let providers: &[(Provider, &str)] = &[
        (Provider::Openai, "OpenAI"),
        (Provider::Anthropic, "Anthropic"),
        (Provider::Gemini, "Google Gemini"),
        (Provider::Deepseek, "DeepSeek"),
        (Provider::Groq, "Groq"),
        (Provider::Ollama, "Ollama (local)"),
    ];

    let stdin = io::stdin();
    let mut reader = stdin.lock();

    loop {
        println!("Select your AI provider:");
        for (i, (provider, label)) in providers.iter().enumerate() {
            let marker = if default == Some(*provider) {
                " [current]"
            } else {
                ""
            };
            println!("  {}. {}{}", i + 1, label, marker);
        }
        println!("  s. Skip setup");
        println!();
        print!("Enter selection [1-6, s]: ");
        io::stdout().flush()?;

        let mut line = String::new();
        reader.read_line(&mut line)?;

        match parse_provider_selection(&line) {
            Ok(result) => return Ok(result),
            Err(e) => {
                println!("{}", e);
                println!();
            }
        }
    }
}

/// Parses model selection input.
///
/// Returns `Ok(None)` if input is empty (user wants default),
/// `Ok(Some(index))` for a valid 1-based numeric index within the options range,
/// or an error for anything else.
pub fn parse_model_selection(input: &str, options: &[ModelPickerOption]) -> Result<Option<usize>> {
    let trimmed = input.trim();
    if trimmed.is_empty() {
        return Ok(None);
    }
    match trimmed.parse::<usize>() {
        Ok(n) if n >= 1 && n <= options.len() => Ok(Some(n - 1)),
        Ok(n) => bail!(
            "Invalid selection '{}'. Please enter a number within 1–{}.",
            n,
            options.len()
        ),
        Err(_) => bail!(
            "Invalid selection '{}'. Please enter a number within 1–{} or press Enter for default.",
            trimmed,
            options.len()
        ),
    }
}

/// Displays the model selection menu for a provider and reads user input in a loop.
///
/// Returns the model ID string for the selected model.
/// If the user presses Enter without input, returns the default model
/// (the one matching `default_model`, or the first model in the list).
pub fn prompt_model_selection(provider: Provider, default_model: Option<&str>) -> Result<String> {
    let options = crate::chat::model_picker_options(provider);
    if options.is_empty() {
        bail!("No models available for provider {:?}.", provider);
    }

    let default_index = default_model
        .and_then(|dm| options.iter().position(|o| o.id == dm))
        .unwrap_or(0);

    let provider_name = match provider {
        Provider::Openai => "OpenAI",
        Provider::Anthropic => "Anthropic",
        Provider::Gemini => "Google Gemini",
        Provider::Deepseek => "DeepSeek",
        Provider::Groq => "Groq",
        Provider::Ollama => "Ollama",
        Provider::Auto => "Auto",
    };

    let stdin = io::stdin();
    let mut reader = stdin.lock();

    loop {
        println!("Select a model for {}:", provider_name);
        for (i, option) in options.iter().enumerate() {
            let marker = if i == default_index { " [default]" } else { "" };
            println!(
                "  {}. {} (ctx={}, {}){}",
                i + 1,
                option.id,
                option.context_window,
                option.description,
                marker,
            );
        }
        println!();
        print!(
            "Enter selection [1-{}] or press Enter for default: ",
            options.len()
        );
        io::stdout().flush()?;

        let mut line = String::new();
        reader.read_line(&mut line)?;

        match parse_model_selection(&line, &options) {
            Ok(None) => return Ok(options[default_index].id.to_string()),
            Ok(Some(idx)) => return Ok(options[idx].id.to_string()),
            Err(e) => {
                println!("{}", e);
                println!();
            }
        }
    }
}

/// Returns `true` if the input contains at least one non-whitespace character.
pub fn validate_api_key(input: &str) -> bool {
    input.chars().any(|c| !c.is_whitespace())
}

/// Prompts the user to enter an API key with masked input.
///
/// Characters are displayed as `*` while typing. Backspace removes the last
/// character. Enter submits the key (must be non-empty). Ctrl+C cancels.
/// Raw mode is always disabled before returning, even on error.
pub fn prompt_api_key(provider: Provider) -> Result<String> {
    let provider_name = match provider {
        Provider::Openai => "OpenAI",
        Provider::Anthropic => "Anthropic",
        Provider::Gemini => "Google Gemini",
        Provider::Deepseek => "DeepSeek",
        Provider::Groq => "Groq",
        Provider::Ollama => "Ollama",
        Provider::Auto => "Auto",
    };

    loop {
        print!("Enter your API key for {}: ", provider_name);
        io::stdout().flush()?;

        let key = read_masked_input()?;

        println!();

        if validate_api_key(&key) {
            return Ok(key);
        }

        println!("API key cannot be empty. Please enter your key.");
        println!();
    }
}
/// Prompts the user for the Ollama host URL with a default value.
///
/// Displays the default URL and returns it if the user presses Enter
/// without typing anything. Otherwise returns the trimmed input.
pub fn prompt_ollama_host(default: &str) -> Result<String> {
    let stdin = io::stdin();
    let mut reader = stdin.lock();

    print!("Enter Ollama host URL [{}]: ", default);
    io::stdout().flush()?;

    let mut line = String::new();
    reader.read_line(&mut line)?;

    let trimmed = line.trim();
    if trimmed.is_empty() {
        Ok(default.to_string())
    } else {
        Ok(trimmed.to_string())
    }
}


/// Reads a line of input with characters masked as `*`.
///
/// Uses crossterm raw mode to capture individual key events.
/// Returns the entered string on Enter, or bails on Ctrl+C.
fn read_masked_input() -> Result<String> {
    let mut buffer = String::new();
    let mut stdout = io::stdout();

    enable_raw_mode()?;

    let result = (|| -> Result<String> {
        loop {
            let event = read()?;
            if let Event::Key(KeyEvent {
                code, modifiers, ..
            }) = event
            {
                if modifiers.contains(KeyModifiers::CONTROL) && code == KeyCode::Char('c') {
                    bail!("Interrupted by user");
                }

                match code {
                    KeyCode::Enter => {
                        return Ok(buffer);
                    }
                    KeyCode::Backspace => {
                        if buffer.pop().is_some() {
                            // Move cursor back, overwrite with space, move back again
                            write!(stdout, "\x08 \x08")?;
                            stdout.flush()?;
                        }
                    }
                    KeyCode::Char(c) => {
                        buffer.push(c);
                        write!(stdout, "*")?;
                        stdout.flush()?;
                    }
                    _ => {}
                }
            }
        }
    })();

    disable_raw_mode()?;
    result
}

/// Masks an API key for display.
///
/// If the key is 10+ characters, shows the first 3 and last 4 with `****...****` in between.
/// Otherwise, masks all but the last 2 characters.
pub fn mask_api_key(key: &str) -> String {
    if key.len() >= 10 {
        let prefix = &key[..3];
        let suffix = &key[key.len() - 4..];
        format!("{}****...****{}", prefix, suffix)
    } else if key.len() > 2 {
        let suffix = &key[key.len() - 2..];
        format!("{}{}", "*".repeat(key.len() - 2), suffix,)
    } else {
        "*".repeat(key.len())
    }
}

/// Formats a human-readable summary of the onboarding result.
pub fn format_summary(result: &OnboardingResult) -> String {
    let provider_name = match result.provider {
        Provider::Openai => "OpenAI",
        Provider::Anthropic => "Anthropic",
        Provider::Gemini => "Google Gemini",
        Provider::Deepseek => "DeepSeek",
        Provider::Groq => "Groq",
        Provider::Ollama => "Ollama",
        Provider::Auto => "Auto",
    };

    let mut lines = Vec::new();
    lines.push("Setup Summary:".to_string());
    lines.push(format!("  Provider: {}", provider_name));
    lines.push(format!("  Model:    {}", result.model));

    if result.provider == Provider::Ollama {
        let host = result
            .ollama_host
            .as_deref()
            .unwrap_or("http://localhost:11434");
        lines.push(format!("  Host:     {}", host));
    } else if let Some(ref key) = result.api_key {
        lines.push(format!("  API Key:  {}", mask_api_key(key)));
    }

    lines.join("\n")
}

/// Runs the interactive onboarding wizard.
///
/// Orchestrates the full flow: provider → credential → model → summary → confirm.
/// If `existing` is `Some`, pre-populates selections for re-run via `setup` command.
/// Returns `OnboardingResult` with `skipped=true` if the user chooses to skip.
pub fn run_onboarding_wizard(existing: Option<&ProfileConfig>) -> Result<OnboardingResult> {
    println!();
    println!("Welcome to zavora! Let's set up your AI provider.");
    println!();

    let default_provider = existing.and_then(|p| p.provider);
    let default_model = existing.and_then(|p| p.model.clone());

    loop {
        // Step 1: Provider selection
        let provider = match prompt_provider_selection(default_provider)? {
            Some(p) => p,
            None => {
                // User chose to skip
                return Ok(OnboardingResult {
                    provider: Provider::Auto,
                    model: String::new(),
                    api_key: None,
                    ollama_host: None,
                    skipped: true,
                });
            }
        };

        // Step 2: Credential (API key or Ollama host)
        let (api_key, ollama_host) = if provider == Provider::Ollama {
            let host = prompt_ollama_host("http://localhost:11434")?;
            (None, Some(host))
        } else {
            let key = prompt_api_key(provider)?;
            (Some(key), None)
        };

        // Step 3: Model selection
        let model = prompt_model_selection(provider, default_model.as_deref())?;

        // Step 4: Build result and show summary
        let result = OnboardingResult {
            provider,
            model,
            api_key,
            ollama_host,
            skipped: false,
        };

        println!();
        println!("{}", format_summary(&result));
        println!();

        // Step 5: Confirm
        print!("Save this configuration? [Y/n]: ");
        io::stdout().flush()?;

        let stdin = io::stdin();
        let mut reader = stdin.lock();
        let mut line = String::new();
        reader.read_line(&mut line)?;

        let answer = line.trim().to_lowercase();
        if answer.is_empty() || answer == "y" || answer == "yes" {
            return Ok(result);
        }

        println!("No problem, let's start over.");
        println!();
    }
}

/// Persists the onboarding result to a TOML config file.
///
/// Creates the parent directory if it doesn't exist, preserves any existing
/// profiles, and updates the "default" profile with the wizard selections.
pub fn persist_onboarding_config(result: &OnboardingResult, config_path: &str) -> Result<()> {
    let path = std::path::Path::new(config_path);

    // Create parent directory (e.g. `.zavora`) if missing.
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).with_context(|| {
            format!(
                "failed to create config directory '{}'",
                parent.display()
            )
        })?;
    }

    // Load existing profiles to preserve other entries.
    let mut profiles_file = load_profiles(config_path).unwrap_or_default();

    // Get or create the "default" profile.
    let profile = profiles_file
        .profiles
        .entry("default".to_string())
        .or_default();

    if result.skipped {
        profile.provider = Some(Provider::Auto);
        profile.model = None;
        profile.api_key = None;
        profile.ollama_host = None;
    } else {
        profile.provider = Some(result.provider);
        profile.model = Some(result.model.clone());
        profile.api_key = result.api_key.clone();
        profile.ollama_host = result.ollama_host.clone();
    }

    let toml_str = toml::to_string_pretty(&profiles_file)
        .context("failed to serialize config to TOML")?;

    std::fs::write(path, toml_str).with_context(|| {
        format!("failed to write config to '{}'", path.display())
    })?;

    Ok(())
}


#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_provider_selection_valid() {
        assert_eq!(
            parse_provider_selection("1").unwrap(),
            Some(Provider::Openai)
        );
        assert_eq!(
            parse_provider_selection("2").unwrap(),
            Some(Provider::Anthropic)
        );
        assert_eq!(
            parse_provider_selection("3").unwrap(),
            Some(Provider::Gemini)
        );
        assert_eq!(
            parse_provider_selection("4").unwrap(),
            Some(Provider::Deepseek)
        );
        assert_eq!(parse_provider_selection("5").unwrap(), Some(Provider::Groq));
        assert_eq!(
            parse_provider_selection("6").unwrap(),
            Some(Provider::Ollama)
        );
    }

    #[test]
    fn test_parse_provider_selection_skip() {
        assert_eq!(parse_provider_selection("s").unwrap(), None);
        assert_eq!(parse_provider_selection("S").unwrap(), None);
    }

    #[test]
    fn test_parse_provider_selection_with_whitespace() {
        assert_eq!(
            parse_provider_selection("  1  ").unwrap(),
            Some(Provider::Openai)
        );
        assert_eq!(parse_provider_selection("  s  ").unwrap(), None);
    }

    #[test]
    fn test_parse_provider_selection_invalid() {
        assert!(parse_provider_selection("0").is_err());
        assert!(parse_provider_selection("7").is_err());
        assert!(parse_provider_selection("abc").is_err());
        assert!(parse_provider_selection("").is_err());
    }

    #[test]
    fn test_parse_model_selection_empty_returns_none() {
        let options = crate::chat::model_picker_options(Provider::Openai);
        assert_eq!(parse_model_selection("", &options).unwrap(), None);
        assert_eq!(parse_model_selection("   ", &options).unwrap(), None);
    }

    #[test]
    fn test_parse_model_selection_valid_index() {
        let options = crate::chat::model_picker_options(Provider::Openai);
        assert_eq!(parse_model_selection("1", &options).unwrap(), Some(0));
        assert_eq!(parse_model_selection("4", &options).unwrap(), Some(3));
    }

    #[test]
    fn test_parse_model_selection_invalid() {
        let options = crate::chat::model_picker_options(Provider::Openai);
        assert!(parse_model_selection("0", &options).is_err());
        assert!(parse_model_selection("99", &options).is_err());
        assert!(parse_model_selection("abc", &options).is_err());
    }

    #[test]
    fn test_validate_api_key_valid() {
        assert!(validate_api_key("sk-abc123"));
        assert!(validate_api_key("a"));
        assert!(validate_api_key("  key  "));
    }

    #[test]
    fn test_validate_api_key_invalid() {
        assert!(!validate_api_key(""));
        assert!(!validate_api_key("   "));
        assert!(!validate_api_key("\t\n"));
    }

    #[test]
    fn test_mask_api_key_long() {
        assert_eq!(mask_api_key("sk-abcdefghij"), "sk-****...****ghij");
        assert_eq!(mask_api_key("1234567890"), "123****...****7890");
    }

    #[test]
    fn test_mask_api_key_short() {
        assert_eq!(mask_api_key("abcde"), "***de");
        assert_eq!(mask_api_key("abc"), "*bc");
    }

    #[test]
    fn test_mask_api_key_very_short() {
        assert_eq!(mask_api_key("ab"), "**");
        assert_eq!(mask_api_key("a"), "*");
        assert_eq!(mask_api_key(""), "");
    }

    #[test]
    fn test_format_summary_cloud_provider() {
        let result = OnboardingResult {
            provider: Provider::Openai,
            model: "gpt-4.1".to_string(),
            api_key: Some("sk-abcdefghijklmnop".to_string()),
            ollama_host: None,
            skipped: false,
        };
        let summary = format_summary(&result);
        assert!(summary.contains("OpenAI"));
        assert!(summary.contains("gpt-4.1"));
        assert!(summary.contains("sk-****...****mnop"));
        assert!(!summary.contains("sk-abcdefghijklmnop"));
    }

    #[test]
    fn test_format_summary_ollama() {
        let result = OnboardingResult {
            provider: Provider::Ollama,
            model: "llama4".to_string(),
            api_key: None,
            ollama_host: Some("http://localhost:11434".to_string()),
            skipped: false,
        };
        let summary = format_summary(&result);
        assert!(summary.contains("Ollama"));
        assert!(summary.contains("llama4"));
        assert!(summary.contains("http://localhost:11434"));
    }
}
