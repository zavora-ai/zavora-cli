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
        "gemini" => 1_000_000,
        "anthropic" => 200_000,
        "openai" => 128_000,
        "deepseek" => 64_000,
        "groq" => 32_000,
        "ollama" => 8_000,
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
            BudgetLevel::Normal => format!("[{}%]", pct),
            BudgetLevel::Warning => format!("[⚠ {}%]", pct),
            BudgetLevel::Critical => format!("[🔴 {}%]", pct),
        }
    }

    /// Detailed display for /usage command.
    pub fn format_usage(&self) -> String {
        let total_tokens = self.total_tokens();
        let window = self.context_window_tokens;
        let pct = (self.utilization() * 100.0) as u32;
        let remaining = window.saturating_sub(total_tokens);

        let mut out = String::new();
        out.push_str(&format!("Context usage: {}/{} tokens ({}%)\n", total_tokens, window, pct));
        out.push_str(&format!("  User:      {} tokens\n", estimate_tokens(self.user_chars)));
        out.push_str(&format!("  Assistant: {} tokens\n", estimate_tokens(self.assistant_chars)));
        out.push_str(&format!("  Tools:     {} tokens\n", estimate_tokens(self.tool_chars)));
        out.push_str(&format!("  System:    {} tokens\n", estimate_tokens(self.system_chars)));
        out.push_str(&format!("  Remaining: {} tokens\n", remaining));

        match self.budget_level() {
            BudgetLevel::Normal => {}
            BudgetLevel::Warning => {
                out.push_str("⚠ Context usage above 80%. Consider using /compact to free space.\n");
            }
            BudgetLevel::Critical => {
                out.push_str("🔴 Context nearly full. Use /compact now or auto-compaction will trigger.\n");
            }
        }
        out
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BudgetLevel {
    Normal,
    Warning,
    Critical,
}
