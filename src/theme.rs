/// Unified UI theme, command palette, and onboarding UX.
///
/// Provides prompt visuals with mode indicators, fuzzy slash command matching,
/// ANSI color helpers, and first-run onboarding help.
use std::fmt;
use std::io::{self, IsTerminal, Write};
use std::path::Path;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};

use crate::checkpoint::CheckpointStore;
use crate::config::RuntimeConfig;
use crate::context::{BudgetLevel, ContextUsage};

// ---------------------------------------------------------------------------
// ANSI color helpers
// ---------------------------------------------------------------------------

#[derive(Clone, Copy)]
pub struct Ansi(&'static str);

impl fmt::Display for Ansi {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        if colors_enabled() {
            formatter.write_str(self.0)
        } else {
            Ok(())
        }
    }
}

pub fn colors_enabled() -> bool {
    std::env::var_os("NO_COLOR").is_none()
        && std::env::var_os("TERM").is_none_or(|term| term != "dumb")
        && (io::stdout().is_terminal() || io::stderr().is_terminal())
}

pub static RESET: Ansi = Ansi("\x1b[0m");
pub static BOLD: Ansi = Ansi("\x1b[1m");
pub static DIM: Ansi = Ansi("\x1b[2m");
pub static CYAN: Ansi = Ansi("\x1b[36m");
pub static GREEN: Ansi = Ansi("\x1b[32m");
pub static YELLOW: Ansi = Ansi("\x1b[33m");
pub static RED: Ansi = Ansi("\x1b[31m");
pub static MAGENTA: Ansi = Ansi("\x1b[35m");
pub static BLUE: Ansi = Ansi("\x1b[34m");
pub static BOLD_CYAN: Ansi = Ansi("\x1b[1;36m");
pub static BOLD_GREEN: Ansi = Ansi("\x1b[1;32m");
pub static BOLD_YELLOW: Ansi = Ansi("\x1b[1;33m");
pub static BOLD_RED: Ansi = Ansi("\x1b[1;31m");
pub static BOLD_MAGENTA: Ansi = Ansi("\x1b[1;35m");
pub static BG_DELETE: Ansi = Ansi("\x1b[48;2;36;25;28m");
pub static BG_INSERT: Ansi = Ansi("\x1b[48;2;24;38;30m");
pub static BG_GUTTER_DELETE: Ansi = Ansi("\x1b[48;2;79;40;40m");
pub static BG_GUTTER_INSERT: Ansi = Ansi("\x1b[48;2;40;67;43m");
pub static CLEAR_LINE: Ansi = Ansi("\x1b[K");

// ---------------------------------------------------------------------------
// Known commands for fuzzy matching
// ---------------------------------------------------------------------------

/// All registered slash commands with descriptions.
pub const COMMAND_PALETTE: &[(&str, &str)] = &[
    ("help", "show command quick reference"),
    ("status", "show active profile/provider/model/session"),
    ("provider", "switch provider and rebuild runtime"),
    ("model", "pick a model interactively or switch by id"),
    ("worker", "switch the everyday coding model"),
    ("planner", "switch the strong planning model"),
    ("planner-provider", "switch the planning provider"),
    ("models", "show model roles and shared quota pools"),
    ("tools", "show active tools and confirmation policy"),
    ("mcp", "show MCP server and tool summary"),
    ("mcps", "show configured MCP servers"),
    ("capabilities", "browse bundled work capability packs"),
    ("skills", "browse invocable workspace skills"),
    ("agents", "browse specialist sub-agents"),
    ("inspect", "inspect the resolved runtime"),
    ("doctor", "check MCP configuration readiness"),
    ("usage", "show context usage and token breakdown"),
    ("compact", "summarize conversation to free context space"),
    (
        "checkpoint",
        "manage conversation snapshots (save|list|restore)",
    ),
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
        let util = usage.utilization();
        let pct_str = if (util * 100.0) as u32 == 0 && util > 0.0 {
            "<1".to_string()
        } else {
            format!("{}", (util * 100.0) as u32)
        };
        let indicator = match usage.budget_level() {
            BudgetLevel::Normal => format!("{DIM}{pct_str}%{RESET}"),
            BudgetLevel::Warning => format!("{BOLD_YELLOW}⚠ {pct_str}%{RESET}"),
            BudgetLevel::Critical => format!("{BOLD_RED}🔴 {pct_str}%{RESET}"),
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
        format!(
            "{BOLD_CYAN}zavora{RESET} {DIM}[{RESET}{}{DIM}]{RESET}{BOLD_CYAN}>{RESET} ",
            parts.join(" ")
        )
    }
}

// ---------------------------------------------------------------------------
// Startup banner
// ---------------------------------------------------------------------------

/// Print a compact runtime surface that remains useful in an 80-column terminal.
pub fn print_startup_banner(cfg: &RuntimeConfig) {
    let version = env!("CARGO_PKG_VERSION");
    let workspace = std::env::current_dir()
        .ok()
        .and_then(|path| {
            path.file_name()
                .map(|name| name.to_string_lossy().into_owned())
        })
        .unwrap_or_else(|| "workspace".to_string());
    println!();
    println!("  {BOLD_CYAN}ZAVORA{RESET} {DIM}v{version} · ADK-Rust 2.0{RESET}");
    println!(
        "  {DIM}workspace{RESET}  {workspace}  {DIM}session{RESET}  {}",
        cfg.session_id
    );
    println!("  {DIM}┌─ MODEL ROUTING ─────────────────────────────────────────────{RESET}");
    println!(
        "  {DIM}│{RESET} {GREEN}WORKER {RESET} {}/{}",
        cfg.worker_provider, cfg.worker_model
    );
    println!(
        "  {DIM}│{RESET} {YELLOW}PLANNER{RESET} {}/{} {DIM}· max {} calls{RESET}",
        cfg.planner_provider, cfg.planner_model, cfg.planner_call_budget
    );
    println!("  {DIM}└──────────────────────────────────────────────────────────────{RESET}");
    println!(
        "  {CYAN}/models{RESET} roles & limits  {DIM}·{RESET}  {CYAN}/help{RESET} commands  {DIM}·{RESET}  {CYAN}/agent{RESET} permissions  {DIM}·{RESET}  {CYAN}/exit{RESET} quit"
    );
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
            let list = cmds
                .iter()
                .map(|c| format!("{CYAN}/{c}{RESET}"))
                .collect::<Vec<_>>()
                .join(", ");
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
    println!(
        "  {DIM}Tip: Commands can be abbreviated — type /ch and press enter to see matches.{RESET}"
    );
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

// ---------------------------------------------------------------------------
// Spinner
// ---------------------------------------------------------------------------

const SPINNER_FRAMES: &[&str] = &["⠋", "⠙", "⠹", "⠸", "⠼", "⠴", "⠦", "⠧", "⠇", "⠏"];

/// Thinking verbs in languages from around the world.
/// Each entry: (verb, language)
const THINKING_VERBS: &[(&str, &str)] = &[
    // African languages
    ("Kufikiri", "Swahili"),
    ("Kũtekereria", "Kikuyu"),
    ("Kuganira", "Kinyarwanda"),
    ("Okulowooza", "Luganda"),
    ("Ero", "Yoruba"),
    ("Iche echiche", "Igbo"),
    ("Nagaani", "Somali"),
    ("Maaloo", "Oromo"),
    ("Mawazo", "Lingala"),
    ("Ho nahana", "Sesotho"),
    ("Ukucabanga", "Zulu"),
    ("Ukucinga", "Xhosa"),
    ("Go nagana", "Setswana"),
    ("Kuganiza", "Chichewa"),
    ("Kufunganya", "Shona"),
    // European languages
    ("Penser", "French"),
    ("Denken", "German"),
    ("Pensando", "Spanish"),
    ("Pensare", "Italian"),
    ("Pensar", "Portuguese"),
    ("Думать", "Russian"),
    ("Myślenie", "Polish"),
    ("Gondolkodás", "Hungarian"),
    ("Σκέψη", "Greek"),
    ("Gândire", "Romanian"),
    ("Tänkande", "Swedish"),
    ("Tenkning", "Norwegian"),
    ("Mõtlemine", "Estonian"),
    ("Přemýšlení", "Czech"),
    ("Myslenie", "Slovak"),
    // Asian languages
    ("考えている", "Japanese"),
    ("생각하는 중", "Korean"),
    ("思考中", "Chinese"),
    ("सोच रहा हूँ", "Hindi"),
    ("ভাবছি", "Bengali"),
    ("คิด", "Thai"),
    ("Suy nghĩ", "Vietnamese"),
    ("Berfikir", "Malay"),
    ("Berpikir", "Indonesian"),
    ("Iniisip", "Filipino"),
    ("සිතමින්", "Sinhala"),
    ("யோசிக்கிறேன்", "Tamil"),
    // Middle Eastern
    ("أفكر", "Arabic"),
    ("فکر کردن", "Persian"),
    ("Düşünmek", "Turkish"),
    ("חושב", "Hebrew"),
    // Other
    ("Whakaaro", "Māori"),
    ("Thinking", "English"),
    ("Cogitare", "Latin"),
];

/// Tips shown below the spinner after a delay.
const SPINNER_TIPS: &[&str] = &[
    "Use /compact to free up context when conversations get long",
    "Use /allow <pattern> to auto-approve tools for this session",
    "Use /delegate <task> to fork an isolated sub-agent",
    "Use file_edit for surgical changes — it's faster than fs_write",
    "Use glob and grep instead of shell find/grep — they're safer and structured",
    "Use /agent to trust all tools for the session (agent mode)",
    "Use /memory recall to check what the agent remembers from past sessions",
    "Use /checkpoint save <label> to snapshot your session",
    "Use /model to switch models mid-conversation",
    "Read-only tools (fs_read, glob, grep) are auto-approved — no confirmation needed",
];

/// Pick a random thinking verb with its language.
fn random_thinking_verb() -> (&'static str, &'static str) {
    use std::collections::hash_map::DefaultHasher;
    use std::hash::{Hash, Hasher};
    let mut hasher = DefaultHasher::new();
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos()
        .hash(&mut hasher);
    std::thread::current().id().hash(&mut hasher);
    let idx = hasher.finish() as usize % THINKING_VERBS.len();
    THINKING_VERBS[idx]
}

fn random_tip() -> &'static str {
    use std::collections::hash_map::DefaultHasher;
    use std::hash::{Hash, Hasher};
    let mut hasher = DefaultHasher::new();
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos()
        .hash(&mut hasher);
    let idx = hasher.finish() as usize % SPINNER_TIPS.len();
    SPINNER_TIPS[idx]
}

/// Global flag to pause the spinner (e.g. during tool confirmation prompts).
static SPINNER_PAUSED: AtomicBool = AtomicBool::new(false);

/// Pause the spinner and clear its line. Call before prompting for user input.
pub fn pause_spinner() {
    SPINNER_PAUSED.store(true, Ordering::SeqCst);
    // Give the spinner thread time to see the flag and clear
    std::thread::sleep(std::time::Duration::from_millis(100));
    eprint!("\r\x1b[2K");
    let _ = io::stderr().flush();
}

/// Resume the spinner after user input is complete.
pub fn resume_spinner() {
    SPINNER_PAUSED.store(false, Ordering::SeqCst);
}

/// A terminal spinner that runs on a background thread.
pub struct Spinner {
    stop: Arc<AtomicBool>,
    handle: Option<std::thread::JoinHandle<()>>,
    token_count: Arc<AtomicU64>,
}

impl Spinner {
    /// Start a spinner with the given message (e.g. "Thinking...").
    /// Picks a random multilingual verb and shows tips after 5 seconds.
    /// Displays elapsed time and token count in Claude Code style:
    ///   ✦ Kufikiri… (thought for 12s · 847 tokens)
    ///     💡 Use /compact to free up context
    pub fn start(message: &str) -> Self {
        let stop = Arc::new(AtomicBool::new(false));
        let stop_clone = stop.clone();
        let token_count = Arc::new(AtomicU64::new(0));
        let token_clone = token_count.clone();
        let msg = message.to_string();
        let (verb, lang) = random_thinking_verb();
        let verb = verb.to_string();
        let lang = lang.to_string();

        let handle = std::thread::spawn(move || {
            let mut i = 0;
            let start = std::time::Instant::now();
            let tip = random_tip();
            let mut phase = 0u8; // 0=initial, 1=verb+stats, 2=verb+stats+tip

            while !stop_clone.load(Ordering::Relaxed) {
                if !SPINNER_PAUSED.load(Ordering::Relaxed) {
                    let frame = SPINNER_FRAMES[i % SPINNER_FRAMES.len()];
                    let elapsed = start.elapsed();
                    let secs = elapsed.as_secs();
                    let tokens = token_clone.load(Ordering::Relaxed);

                    let new_phase = if secs >= 8 {
                        2
                    } else if secs >= 3 {
                        1
                    } else {
                        0
                    };

                    // Clear previous lines when transitioning
                    if new_phase > phase {
                        if phase >= 1 {
                            // Clear tip line if it existed
                            eprint!("\n\r\x1b[2K\x1b[1A");
                        }
                        phase = new_phase;
                    }

                    match phase {
                        0 => {
                            eprint!("\r\x1b[2K{DIM}{frame} {msg}{RESET}");
                        }
                        1 => {
                            let stats = format_stats(secs, tokens);
                            eprint!(
                                "\r\x1b[2K{CYAN}✦{RESET} {DIM}{verb}…{RESET} {DIM}(Thinking in {lang} · {stats}){RESET}"
                            );
                        }
                        _ => {
                            let stats = format_stats(secs, tokens);
                            eprint!(
                                "\r\x1b[2K{CYAN}✦{RESET} {DIM}{verb}…{RESET} {DIM}(Thinking in {lang} · {stats}){RESET}"
                            );
                            eprint!("\n\r\x1b[2K  {DIM}💡 {tip}{RESET}");
                            eprint!("\x1b[1A");
                        }
                    }
                    let _ = io::stderr().flush();
                }
                std::thread::sleep(std::time::Duration::from_millis(80));
                i += 1;
            }
            // Clear spinner lines
            eprint!("\r\x1b[2K");
            if phase >= 2 {
                eprint!("\n\r\x1b[2K\x1b[1A");
            }
            let _ = io::stderr().flush();
        });

        Self {
            stop,
            handle: Some(handle),
            token_count,
        }
    }

    /// Update the token count displayed by the spinner.
    pub fn set_tokens(&self, count: u64) {
        self.token_count.store(count, Ordering::Relaxed);
    }

    /// Add to the token count.
    pub fn add_tokens(&self, delta: u64) {
        self.token_count.fetch_add(delta, Ordering::Relaxed);
    }

    /// Stop the spinner and clear the line.
    pub fn stop(mut self) {
        self.stop.store(true, Ordering::Relaxed);
        if let Some(h) = self.handle.take() {
            let _ = h.join();
        }
    }
}

fn format_stats(secs: u64, tokens: u64) -> String {
    let time = if secs < 60 {
        format!("{}s", secs)
    } else {
        format!("{}m{}s", secs / 60, secs % 60)
    };
    if tokens > 0 {
        format!("{} · {} tokens", time, tokens)
    } else {
        time
    }
}

impl Drop for Spinner {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        if let Some(h) = self.handle.take() {
            let _ = h.join();
        }
    }
}
