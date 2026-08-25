//! Context estimation, utilization, formatting, and boundary tests.

// ---------------------------------------------------------------------------
// Context usage and token counting tests
// ---------------------------------------------------------------------------

use crate::context::*;

#[test]
fn test_estimate_tokens() {
    assert_eq!(estimate_tokens(0), 0);
    assert_eq!(estimate_tokens(40), 10); // 40/4 = 10, rounded to 10
    assert_eq!(estimate_tokens(100), 30); // 100/4 = 25, (25+5)/10*10 = 30
    assert_eq!(estimate_tokens(400), 100); // 400/4 = 100
}

#[test]
fn test_tokens_to_chars() {
    assert_eq!(tokens_to_chars(100), 400);
    assert_eq!(tokens_to_chars(0), 0);
}

#[test]
fn test_context_usage_normal() {
    let usage = ContextUsage {
        user_chars: 1000,
        assistant_chars: 2000,
        tool_chars: 500,
        system_chars: 500,
        context_window_tokens: 128_000,
        api_total_tokens: 0,
        event_count: 0,
    };
    assert_eq!(usage.total_chars(), 4000);
    assert_eq!(usage.budget_level(), BudgetLevel::Normal);
    assert!(usage.utilization() < WARN_THRESHOLD);
    let indicator = usage.prompt_indicator();
    assert!(!indicator.contains("⚠"));
    assert!(!indicator.contains("🔴"));
}

#[test]
fn test_context_usage_warning() {
    // 80% of 1000 tokens via api_total_tokens
    let usage = ContextUsage {
        user_chars: 1600,
        assistant_chars: 1600,
        tool_chars: 0,
        system_chars: 0,
        context_window_tokens: 1000,
        api_total_tokens: 800,
        event_count: 0,
    };
    assert_eq!(usage.budget_level(), BudgetLevel::Warning);
    assert!(usage.prompt_indicator().contains("⚠"));
}

#[test]
fn test_context_usage_critical() {
    // 95% of 1000 tokens via api_total_tokens
    let usage = ContextUsage {
        user_chars: 1900,
        assistant_chars: 1900,
        tool_chars: 0,
        system_chars: 0,
        context_window_tokens: 1000,
        api_total_tokens: 950,
        event_count: 0,
    };
    assert_eq!(usage.budget_level(), BudgetLevel::Critical);
    assert!(usage.prompt_indicator().contains("🔴"));
}

#[test]
fn test_context_usage_format() {
    let usage = ContextUsage {
        user_chars: 4000,
        assistant_chars: 8000,
        tool_chars: 2000,
        system_chars: 1000,
        context_window_tokens: 128_000,
        api_total_tokens: 0,
        event_count: 0,
    };
    let output = usage.format_usage();
    assert!(output.contains("Context"));
    assert!(output.contains("User:"));
    assert!(output.contains("Assistant:"));
    assert!(output.contains("Tools:"));
    assert!(output.contains("Remaining:"));
}

#[test]
fn test_default_context_window() {
    assert_eq!(default_context_window("gemini"), 1_048_576);
    assert_eq!(default_context_window("anthropic"), 1_000_000);
    assert_eq!(default_context_window("openai"), 1_000_000);
    assert_eq!(default_context_window("deepseek"), 128_000);
    assert_eq!(default_context_window("groq"), 131_072);
    assert_eq!(default_context_window("unknown"), 128_000);

    // Model-specific overrides
    assert_eq!(model_context_window("gpt-5-mini", "openai"), 400_000);
    assert_eq!(model_context_window("gpt-4.1", "openai"), 1_000_000);
    assert_eq!(model_context_window("o3-mini", "openai"), 200_000);
    assert_eq!(
        model_context_window("claude-sonnet-4-20250514", "anthropic"),
        1_000_000
    );
    assert_eq!(model_context_window("deepseek-chat", "deepseek"), 128_000);
    assert_eq!(
        model_context_window("llama-4-scout-17b-16e-instruct", "groq"),
        131_072
    );
}

#[test]
fn test_context_usage_zero_window() {
    let usage = ContextUsage {
        user_chars: 100,
        assistant_chars: 0,
        tool_chars: 0,
        system_chars: 0,
        context_window_tokens: 0,
        api_total_tokens: 0,
        event_count: 0,
    };
    assert_eq!(usage.utilization(), 0.0);
    assert_eq!(usage.budget_level(), BudgetLevel::Normal);
}
