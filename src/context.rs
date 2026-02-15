/// Token counting and context usage tracking.
///
/// Uses a char/4 heuristic (same approach as Q CLI) — no external tokenizer
/// dependency. Rounded to nearest 10 to avoid false precision.

// ---------------------------------------------------------------------------
// Token counting
// ---------------------------------------------------------------------------

/// Chars-per-token ratio used for estimation.
pub const TOKEN_CHAR_RATIO: usize = 4;

/// Estimate token count from character length. Rounds to nearest 10.
pub fn estimate_tokens(char_count: usize) -> usize {
    (char_count / TOKEN_CHAR_RATIO + 5) / 10 * 10
}

/// Convert a token count back to approximate character count.
pub const fn tokens_to_chars(tokens: usize) -> usize {
    tokens * TOKEN_CHAR_RATIO
}

// ---------------------------------------------------------------------------
// Context usage tracking
// ---------------------------------------------------------------------------

/// Default context window sizes per provider (tokens).
pub fn default_context_window(provider: &str) -> usize {
    // Provider-level fallback; prefer model_context_window() when model name is available.
    match provider {
        "gemini" => 1_048_576,
        "anthropic" => 1_000_000,
        "openai" => 1_000_000,
        "deepseek" => 128_000,
        "groq" => 131_072,
        "ollama" => 128_000,
        _ => 128_000,
    }
}

/// Context window for a specific model. Falls back to provider default.
pub fn model_context_window(model: &str, provider: &str) -> usize {
    match model {
        // OpenAI
        "gpt-4.1" | "gpt-4.1-mini" | "gpt-4.1-nano" => 1_000_000,
        m if m.starts_with("gpt-5") => 400_000,
        "o3-mini" | "o3" | "o4-mini" => 200_000,
        // Anthropic
        "claude-opus-4-6" => 1_000_000,
        m if m.starts_with("claude-sonnet-4") => 1_000_000,
        m if m.starts_with("claude-3-5-haiku") => 200_000,
        // Gemini
        m if m.contains("gemini-2.5") => 1_048_576,
        m if m.contains("gemini-3") => 1_048_576,
        // DeepSeek
        m if m.starts_with("deepseek") => 128_000,
        // Groq-hosted models
        "llama-3.3-70b-versatile" => 131_072,
        "llama-4-scout-17b-16e-instruct" => 131_072,
        "deepseek-r1-distill-llama-70b" => 128_000,
        _ => default_context_window(provider),
    }
}

/// Warning thresholds as fractions of context window.
pub const WARN_THRESHOLD: f64 = 0.80;
pub const CRITICAL_THRESHOLD: f64 = 0.90;

/// Snapshot of context usage at a point in time.
#[derive(Debug, Clone)]
pub struct ContextUsage {
    pub user_chars: usize,
    pub assistant_chars: usize,
    pub tool_chars: usize,
    pub system_chars: usize,
    pub context_window_tokens: usize,
    /// Actual total token count from the API's last usage_metadata (includes system prompt, tools, history).
    pub api_total_tokens: usize,
    /// Number of session events.
    pub event_count: usize,
}

/// Estimated overhead for system prompt + tool declarations not captured in session events.
const PROMPT_OVERHEAD_TOKENS: usize = 1500;

impl ContextUsage {
    pub fn total_chars(&self) -> usize {
        self.user_chars + self.assistant_chars + self.tool_chars + self.system_chars
    }

    pub fn total_tokens(&self) -> usize {
        if self.api_total_tokens > 0 {
            self.api_total_tokens
        } else {
            estimate_tokens(self.total_chars()) + PROMPT_OVERHEAD_TOKENS
        }
    }

    pub fn utilization(&self) -> f64 {
        if self.context_window_tokens == 0 {
            return 0.0;
        }
        self.total_tokens() as f64 / self.context_window_tokens as f64
    }

    pub fn budget_level(&self) -> BudgetLevel {
        let util = self.utilization();
        if util >= CRITICAL_THRESHOLD {
            BudgetLevel::Critical
        } else if util >= WARN_THRESHOLD {
            BudgetLevel::Warning
        } else {
            BudgetLevel::Normal
        }
    }

    /// Short indicator string for prompt display (e.g. "[<1%]", "[72%]", "[⚠ 85%]", "[🔴 93%]").
    pub fn prompt_indicator(&self) -> String {
        let util = self.utilization();
        let pct = (util * 100.0) as u32;
        match self.budget_level() {
            BudgetLevel::Normal if pct == 0 && util > 0.0 => "<1%".to_string(),
            BudgetLevel::Normal => format!("{}%", pct),
            BudgetLevel::Warning => format!("⚠ {}%", pct),
            BudgetLevel::Critical => format!("🔴 {}%", pct),
        }
    }

    /// Detailed display for /usage command.
    pub fn format_usage(&self) -> String {
        let total_tokens = self.total_tokens();
        let window = self.context_window_tokens;
        let pct = (self.utilization() * 100.0) as u32;
        let remaining = window.saturating_sub(total_tokens);

        let pct_color = match self.budget_level() {
            BudgetLevel::Normal => "\x1b[32m",   // green
            BudgetLevel::Warning => "\x1b[1;33m", // bold yellow
            BudgetLevel::Critical => "\x1b[1;31m", // bold red
        };
        let d = "\x1b[2m"; // dim
        let r = "\x1b[0m"; // reset
        let b = "\x1b[1m"; // bold

        let pct_display = if pct == 0 && self.utilization() > 0.0 {
            "<1".to_string()
        } else {
            pct.to_string()
        };

        let mut out = String::new();
        out.push_str(&format!("\n  {b}Context{r}  {pct_color}{total_tokens}/{window} tokens ({pct_display}%){r}\n\n"));
        out.push_str(&format!("  {d}User:{r}      {:>6} tokens\n", estimate_tokens(self.user_chars)));
        out.push_str(&format!("  {d}Assistant:{r}  {:>6} tokens\n", estimate_tokens(self.assistant_chars)));
        out.push_str(&format!("  {d}Tools:{r}     {:>6} tokens\n", estimate_tokens(self.tool_chars)));
        out.push_str(&format!("  {d}System:{r}    {:>6} tokens\n", estimate_tokens(self.system_chars)));
        out.push_str(&format!("  {d}Overhead:{r}  {:>6} tokens {d}(system prompt + tool decls){r}\n", PROMPT_OVERHEAD_TOKENS));
        out.push_str(&format!("  {d}Remaining:{r} {:>6} tokens\n", remaining));

        out.push_str(&format!("\n  {d}Events:{r}    {:>6}\n", self.event_count));
        out.push_str(&format!("  {d}Chars:{r}     {:>6}\n", self.total_chars()));
        if self.api_total_tokens > 0 {
            out.push_str(&format!("  {d}API tokens:{r}{:>6} {d}(from provider){r}\n", self.api_total_tokens));
        }

        match self.budget_level() {
            BudgetLevel::Normal => {}
            BudgetLevel::Warning => {
                out.push_str(&format!("\n  {pct_color}⚠ Above 80% — consider /compact to free space.{r}\n"));
            }
            BudgetLevel::Critical => {
                out.push_str(&format!("\n  {pct_color}🔴 Nearly full — use /compact now.{r}\n"));
            }
        }
        out.push('\n');
        out
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BudgetLevel {
    Normal,
    Warning,
    Critical,
}

// ---------------------------------------------------------------------------
// Compute from session events
// ---------------------------------------------------------------------------

use adk_rust::Event;

/// Build a `ContextUsage` snapshot from session events and provider name.
pub fn compute_context_usage(events: &[Event], provider: &str, model: &str) -> ContextUsage {
    let mut user_chars = 0usize;
    let mut assistant_chars = 0usize;
    let mut tool_chars = 0usize;
    let mut system_chars = 0usize;
    let mut api_total_tokens = 0usize;

    for event in events {
        let (mut text_chars, mut fn_chars) = (0usize, 0usize);
        if let Some(content) = &event.llm_response.content {
            for part in &content.parts {
                match part {
                    adk_rust::Part::Text { text } => text_chars += text.len(),
                    adk_rust::Part::FunctionCall { name, args, .. } => {
                        fn_chars += name.len() + args.to_string().len();
                    }
                    adk_rust::Part::FunctionResponse { function_response, .. } => {
                        fn_chars += function_response.name.len() + function_response.response.to_string().len();
                    }
                    _ => {}
                }
            }
        }

        // Track the latest API-reported total token count (includes system prompt, tools, full history)
        if let Some(meta) = &event.llm_response.usage_metadata {
            if meta.total_token_count > 0 {
                api_total_tokens = meta.total_token_count as usize;
            }
        }

        match event.author.as_str() {
            "user" => user_chars += text_chars + fn_chars,
            "system" => system_chars += text_chars + fn_chars,
            _ => {
                assistant_chars += text_chars;
                tool_chars += fn_chars;
            }
        }
    }

    ContextUsage {
        user_chars,
        assistant_chars,
        tool_chars,
        system_chars,
        context_window_tokens: model_context_window(model, provider),
        api_total_tokens,
        event_count: events.len(),
    }
}
