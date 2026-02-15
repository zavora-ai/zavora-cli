/// Unified UI theme, command palette, and onboarding UX.
///
/// Provides prompt visuals with mode indicators, fuzzy slash command matching,
/// ANSI color helpers, and first-run onboarding help.
use std::path::Path;

use crate::checkpoint::CheckpointStore;
use crate::context::{BudgetLevel, ContextUsage};

// ---------------------------------------------------------------------------
// ANSI color helpers
// ---------------------------------------------------------------------------

pub const RESET: &str = "\x1b[0m";
pub const BOLD: &str = "\x1b[1m";
pub const DIM: &str = "\x1b[2m";
pub const CYAN: &str = "\x1b[36m";
pub const GREEN: &str = "\x1b[32m";
pub const YELLOW: &str = "\x1b[33m";
pub const RED: &str = "\x1b[31m";
pub const MAGENTA: &str = "\x1b[35m";
pub const BLUE: &str = "\x1b[34m";
pub const BOLD_CYAN: &str = "\x1b[1;36m";
pub const BOLD_GREEN: &str = "\x1b[1;32m";
pub const BOLD_YELLOW: &str = "\x1b[1;33m";
pub const BOLD_RED: &str = "\x1b[1;31m";
pub const BOLD_MAGENTA: &str = "\x1b[1;35m";

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

/// Build the interactive prompt string with mode indicators and color.
pub fn build_prompt(
    checkpoint_store: &CheckpointStore,
    context_usage: Option<&ContextUsage>,
) -> String {
    let mut parts = Vec::new();

    // Budget indicator with color
    if let Some(usage) = context_usage {
        let pct = (usage.utilization() * 100.0) as u32;
        let indicator = match usage.budget_level() {
            BudgetLevel::Normal => format!("{DIM}{}%{RESET}", pct),
            BudgetLevel::Warning => format!("{BOLD_YELLOW}⚠ {}%{RESET}", pct),
            BudgetLevel::Critical => format!("{BOLD_RED}🔴 {}%{RESET}", pct),
        };
        parts.push(indicator);
    }

    // Tangent mode indicator
    if checkpoint_store.in_tangent() {
        parts.push(format!("{MAGENTA}↯tangent{RESET}"));
    }

    if parts.is_empty() {
        format!("{BOLD_CYAN}zavora>{RESET} ")
    } else {
        format!("{BOLD_CYAN}zavora{RESET} {DIM}[{RESET}{}{DIM}]{RESET}{BOLD_CYAN}>{RESET} ", parts.join(" "))
    }
}

// ---------------------------------------------------------------------------
// Startup banner
// ---------------------------------------------------------------------------

/// Print the chat startup banner.
pub fn print_startup_banner(provider: &str, model: &str) {
    println!();
    println!("  {BOLD_CYAN}zavora-cli{RESET} {DIM}v{}{RESET}", env!("CARGO_PKG_VERSION"));
    println!("  {DIM}Provider:{RESET} {GREEN}{provider}{RESET}  {DIM}Model:{RESET} {GREEN}{model}{RESET}");
    println!();
    println!("  Type a message to chat, {CYAN}/help{RESET} for commands, {CYAN}/exit{RESET} to quit.");
    println!();
}

// ---------------------------------------------------------------------------
// Fuzzy command matching
// ---------------------------------------------------------------------------

/// Find the best fuzzy match for a command prefix among known commands.
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
        FuzzyResult::Exact(cmd) => Some(format!("Did you mean {CYAN}/{cmd}{RESET}?")),
        FuzzyResult::Ambiguous(cmds) => {
            let list = cmds.iter().map(|c| format!("{CYAN}/{c}{RESET}")).collect::<Vec<_>>().join(", ");
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
    println!("  {BOLD}Welcome to zavora-cli!{RESET} Here's how to get started:");
    println!();
    println!("  {DIM}1.{RESET} Type a message to chat with the AI agent");
    println!("  {DIM}2.{RESET} Use {CYAN}/help{RESET} to see all available commands");
    println!("  {DIM}3.{RESET} Use {CYAN}/provider{RESET} <name> to switch AI providers");
    println!("  {DIM}4.{RESET} Use {CYAN}/model{RESET} to pick a model interactively");
    println!("  {DIM}5.{RESET} Use {CYAN}/tools{RESET} to see available tools");
    println!();
    println!("  {DIM}Tip: Commands can be abbreviated — type /ch and press enter to see matches.{RESET}");
    println!();
}

/// Format the command palette for display.
pub fn format_command_palette() -> String {
    let mut out = String::from("Command palette:\n");
    for (name, desc) in COMMAND_PALETTE {
        out.push_str(&format!("  {CYAN}/{name:<12}{RESET} {DIM}{desc}{RESET}\n"));
    }
    out
}
