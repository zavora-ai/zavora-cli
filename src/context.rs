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
    match provider {
        "gemini" => 2_000_000,
        "anthropic" => 200_000,
        "openai" => 1_000_000,
        "deepseek" => 64_000,
        "groq" => 128_000,
        "ollama" => 128_000,
        _ => 128_000,
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
}

impl ContextUsage {
    pub fn total_chars(&self) -> usize {
        self.user_chars + self.assistant_chars + self.tool_chars + self.system_chars
    }

    pub fn total_tokens(&self) -> usize {
        estimate_tokens(self.total_chars())
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

    /// Short indicator string for prompt display (e.g. "[72%]", "[⚠ 85%]", "[🔴 93%]").
    pub fn prompt_indicator(&self) -> String {
        let pct = (self.utilization() * 100.0) as u32;
        match self.budget_level() {
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

        let mut out = String::new();
        out.push_str(&format!("\n  {b}Context{r}  {pct_color}{total_tokens}/{window} tokens ({pct}%){r}\n\n"));
        out.push_str(&format!("  {d}User:{r}      {:>6} tokens\n", estimate_tokens(self.user_chars)));
        out.push_str(&format!("  {d}Assistant:{r}  {:>6} tokens\n", estimate_tokens(self.assistant_chars)));
        out.push_str(&format!("  {d}Tools:{r}     {:>6} tokens\n", estimate_tokens(self.tool_chars)));
        out.push_str(&format!("  {d}System:{r}    {:>6} tokens\n", estimate_tokens(self.system_chars)));
        out.push_str(&format!("  {d}Remaining:{r} {:>6} tokens\n", remaining));

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
pub fn compute_context_usage(events: &[Event], provider: &str) -> ContextUsage {
    let mut user_chars = 0usize;
    let mut assistant_chars = 0usize;
    let mut tool_chars = 0usize;
    let mut system_chars = 0usize;

    for event in events {
        let text_len = event
            .llm_response
            .content
            .as_ref()
            .map(|c| {
                c.parts
                    .iter()
                    .map(|p| match p {
                        adk_rust::Part::Text { text } => text.len(),
                        _ => 0,
                    })
                    .sum::<usize>()
            })
            .unwrap_or(0);

        match event.author.as_str() {
            "user" => user_chars += text_len,
            "system" => system_chars += text_len,
            _ => {
                // Check if this event has tool-related content (function calls/responses)
                let has_tool_parts = event
                    .llm_response
                    .content
                    .as_ref()
                    .map(|c| {
                        c.parts.iter().any(|p| {
                            matches!(
                                p,
                                adk_rust::Part::FunctionCall { .. }
                                    | adk_rust::Part::FunctionResponse { .. }
                            )
                        })
                    })
                    .unwrap_or(false);
                if has_tool_parts {
                    tool_chars += text_len;
                } else {
                    assistant_chars += text_len;
                }
            }
        }
    }

    ContextUsage {
        user_chars,
        assistant_chars,
        tool_chars,
        system_chars,
        context_window_tokens: default_context_window(provider),
    }
}
