/// Unified UI theme, command palette, and onboarding UX.
///
/// Provides prompt visuals with mode indicators, fuzzy slash command matching,
/// and first-run onboarding help.
use std::path::Path;

use crate::checkpoint::CheckpointStore;
use crate::context::ContextUsage;

// ---------------------------------------------------------------------------
// Known commands for fuzzy matching
// ---------------------------------------------------------------------------

/// All registered slash commands with descriptions.
pub const COMMAND_PALETTE: &[(&str, &str)] = &[
    ("help", "show command quick reference"),
    ("status", "show active profile/provider/model/session"),
    ("provider", "switch provider and rebuild runtime"),
    ("model", "pick a model interactively or switch by id"),
    ("tools", "show active tools and confirmation policy"),
    ("mcp", "show MCP server and tool summary"),
    ("usage", "show context usage and token breakdown"),
    ("compact", "summarize conversation to free context space"),
    ("checkpoint", "manage conversation snapshots (save|list|restore)"),
    ("tangent", "enter/exit exploratory branch"),
    ("todos", "view/delete/clear-finished task lists"),
    ("delegate", "(experimental) run isolated sub-agent task"),
    ("exit", "end interactive chat"),
];

// ---------------------------------------------------------------------------
// Prompt builder
// ---------------------------------------------------------------------------

/// Build the interactive prompt string with mode indicators.
pub fn build_prompt(
    checkpoint_store: &CheckpointStore,
    context_usage: Option<&ContextUsage>,
) -> String {
    let mut parts = Vec::new();

    // Budget indicator
    if let Some(usage) = context_usage {
        let indicator = usage.prompt_indicator();
        if !indicator.is_empty() {
            parts.push(indicator);
        }
    }

    // Tangent mode indicator
    if checkpoint_store.in_tangent() {
        parts.push("↯tangent".to_string());
    }

    if parts.is_empty() {
        "zavora> ".to_string()
    } else {
        format!("zavora [{}]> ", parts.join(" "))
    }
}

// ---------------------------------------------------------------------------
// Fuzzy command matching
// ---------------------------------------------------------------------------

/// Find the best fuzzy match for a command prefix among known commands.
/// Returns the matched command name if exactly one command starts with the input,
/// or a list of candidates if ambiguous.
pub fn fuzzy_match_command(input: &str) -> FuzzyResult {
    let lower = input.to_ascii_lowercase();
    let matches: Vec<&str> = COMMAND_PALETTE
        .iter()
        .filter(|(name, _)| name.starts_with(&lower))
        .map(|(name, _)| *name)
        .collect();

    match matches.len() {
        0 => FuzzyResult::NoMatch,
        1 => FuzzyResult::Exact(matches[0].to_string()),
        _ => FuzzyResult::Ambiguous(matches.iter().map(|s| s.to_string()).collect()),
    }
}

/// Result of fuzzy command matching.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FuzzyResult {
    /// No commands match the prefix.
    NoMatch,
    /// Exactly one command matches.
    Exact(String),
    /// Multiple commands match — ambiguous.
    Ambiguous(Vec<String>),
}

/// Format a "did you mean?" suggestion for an unknown command.
pub fn suggest_command(input: &str) -> Option<String> {
    match fuzzy_match_command(input) {
        FuzzyResult::Exact(cmd) => Some(format!("Did you mean /{cmd}?")),
        FuzzyResult::Ambiguous(cmds) => {
            let list = cmds.iter().map(|c| format!("/{c}")).collect::<Vec<_>>().join(", ");
            Some(format!("Did you mean one of: {list}?"))
        }
        FuzzyResult::NoMatch => None,
    }
}

// ---------------------------------------------------------------------------
// Onboarding
// ---------------------------------------------------------------------------

/// Check if this is the first run (no .zavora directory exists).
pub fn is_first_run(workspace: &Path) -> bool {
    !workspace.join(".zavora").exists()
}

/// Print first-run onboarding help.
pub fn print_onboarding() {
    println!("Welcome to zavora-cli! Here's how to get started:");
    println!();
    println!("  1. Type a message to chat with the AI agent");
    println!("  2. Use /help to see all available commands");
    println!("  3. Use /provider <name> to switch AI providers");
    println!("  4. Use /model to pick a model interactively");
    println!("  5. Use /tools to see available tools");
    println!();
    println!("Tip: Commands can be abbreviated — type /ch and press enter to see matches.");
    println!();
}

/// Format the command palette for display.
pub fn format_command_palette() -> String {
    let mut out = String::from("Command palette:\n");
    for (name, desc) in COMMAND_PALETTE {
        out.push_str(&format!("  /{name:<12} {desc}\n"));
    }
    out
}
