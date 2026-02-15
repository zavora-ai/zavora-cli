use anyhow;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ErrorCategory {
    Provider,
    Session,
    Tooling,
    Input,
    Internal,
}

impl ErrorCategory {
    pub fn code(self) -> &'static str {
        match self {
            ErrorCategory::Provider => "PROVIDER",
            ErrorCategory::Session => "SESSION",
            ErrorCategory::Tooling => "TOOLING",
            ErrorCategory::Input => "INPUT",
            ErrorCategory::Internal => "INTERNAL",
        }
    }

    pub fn hint(self) -> &'static str {
        match self {
            ErrorCategory::Provider => {
                "Set provider credentials (for example OPENAI_API_KEY) or run with --provider ollama."
            }
            ErrorCategory::Session => {
                "Check --session-backend/--session-db-url and run migrate for sqlite sessions."
            }
            ErrorCategory::Tooling => {
                "Review tool configuration and retry with RUST_LOG=info for detailed tool/runtime logs."
            }
            ErrorCategory::Input => "Run zavora-cli --help and correct command arguments.",
            ErrorCategory::Internal => {
                "Retry with RUST_LOG=debug. If it persists, capture logs and open an issue."
            }
        }
    }
}

pub fn categorize_error(err: &anyhow::Error) -> ErrorCategory {
    let msg = format!("{err:#}").to_ascii_lowercase();

    if msg.contains("api_key")
        || msg.contains("no provider could be auto-detected")
        || msg.contains("provider")
    {
        return ErrorCategory::Provider;
    }

    if msg.contains("--force")
        || msg.contains("destructive")
        || msg.contains("invalid value")
        || msg.contains("unknown argument")
        || msg.contains("failed to read input")
        || msg.contains("profile")
    {
        return ErrorCategory::Input;
    }

    if msg.contains("session") || msg.contains("sqlite") || msg.contains("migrate") {
        return ErrorCategory::Session;
    }

    if msg.contains("tool") || msg.contains("mcp") || msg.contains("retrieval") {
        return ErrorCategory::Tooling;
    }

    ErrorCategory::Internal
}

pub fn format_cli_error(err: &anyhow::Error, show_sensitive_config: bool) -> String {
    let category = categorize_error(err);
    let rendered_error = render_error_message(err, show_sensitive_config);
    format!(
        "[{}] {}\nHint: {}",
        category.code(),
        rendered_error,
        category.hint()
    )
}

pub fn render_error_message(err: &anyhow::Error, show_sensitive_config: bool) -> String {
    if show_sensitive_config {
        err.to_string()
    } else {
        redact_sensitive_text(&err.to_string())
    }
}

pub fn redact_sensitive_text(text: &str) -> String {
    redact_sqlite_urls(text)
}

pub fn redact_sqlite_urls(text: &str) -> String {
    const SQLITE_PREFIX: &str = "sqlite:";
    let mut out = String::with_capacity(text.len());
    let mut cursor = 0usize;

    while let Some(offset) = text[cursor..].find(SQLITE_PREFIX) {
        let start = cursor + offset;
        out.push_str(&text[cursor..start]);

        let remainder = &text[start..];
        let end = remainder
            .find(|ch: char| {
                ch.is_whitespace()
                    || matches!(
                        ch,
                        '"' | '\'' | '(' | ')' | '[' | ']' | '{' | '}' | ',' | ';'
                    )
            })
            .unwrap_or(remainder.len());
        let token = &remainder[..end];
        out.push_str(&redact_sqlite_url_value(token));
        cursor = start + end;
    }

    out.push_str(&text[cursor..]);
    out
}

pub fn redact_sqlite_url_value(value: &str) -> String {
    if value.starts_with("sqlite://") {
        "sqlite://[REDACTED]".to_string()
    } else if value.starts_with("sqlite:") {
        "sqlite:[REDACTED]".to_string()
    } else {
        value.to_string()
    }
}
