//! Canonical metadata for commands available in interactive sessions.
//!
//! Parsing and execution remain owned by the classic-chat and TUI surfaces,
//! but names, aliases, usage, descriptions, categories, and surface support
//! live here so help, completion, and command palettes cannot drift apart.

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum InteractiveCommandKind {
    Help,
    Status,
    Clear,
    Mode,
    Tools,
    Shell,
    Capabilities,
    Essentials,
    Mcp,
    Skills,
    Plugins,
    Instructions,
    Agents,
    Teams,
    Inspect,
    Doctor,
    Usage,
    Sessions,
    NewSession,
    Copy,
    Mouse,
    Export,
    Compact,
    Checkpoint,
    Tangent,
    Undo,
    Todos,
    Delegate,
    Parallel,
    Spawn,
    Runs,
    AgentSend,
    AgentCancel,
    Ralph,
    Models,
    Provider,
    Model,
    Worker,
    PlannerProvider,
    Planner,
    Width,
    Activity,
    Keys,
    AutoCompact,
    Allow,
    Deny,
    Agent,
    Memory,
    Time,
    Exit,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum InteractiveCommandSurface {
    Shared,
    TuiOnly,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct InteractiveCommandSpec {
    pub(crate) kind: InteractiveCommandKind,
    /// Canonical command with its leading slash, for display and completion.
    pub(crate) name: &'static str,
    /// Canonical command without its leading slash, for parsing and compatibility.
    pub(crate) bare_name: &'static str,
    pub(crate) aliases: &'static [&'static str],
    pub(crate) usage: &'static str,
    pub(crate) description: &'static str,
    pub(crate) category: &'static str,
    pub(crate) surface: InteractiveCommandSurface,
    missing_argument_usage: &'static str,
}

pub(crate) const COMMAND_CATEGORIES: &[&str] = &[
    "Workspace",
    "Runtime",
    "Session",
    "Work",
    "Models",
    "Settings",
    "Safety",
    "Utilities",
];

impl InteractiveCommandSpec {
    pub(crate) fn missing_argument_usage(self) -> &'static str {
        if self.missing_argument_usage.is_empty() {
            self.usage
        } else {
            self.missing_argument_usage
        }
    }
}

macro_rules! command_surface {
    (shared) => {
        InteractiveCommandSurface::Shared
    };
    (tui_only) => {
        InteractiveCommandSurface::TuiOnly
    };
}

macro_rules! define_classic_palette {
    (@build [$($entries:expr,)*]) => {
        /// Compatibility view used by the classic theme's fuzzy matcher.
        pub(crate) const CLASSIC_COMMAND_PALETTE: &[(&str, &str)] = &[$($entries,)*];
    };
    (@build [$($entries:expr,)*]
        shared($kind:ident, $name:literal, [$($alias:literal),*], $usage:literal,
            $description:literal, $category:literal, $missing:literal);
        $($rest:tt)*) => {
        define_classic_palette!(@build [
            $($entries,)*
            ($name, $description),
            $(($alias, $description),)*
        ] $($rest)*);
    };
    (@build [$($entries:expr,)*]
        tui_only($kind:ident, $name:literal, [$($alias:literal),*], $usage:literal,
            $description:literal, $category:literal, $missing:literal);
        $($rest:tt)*) => {
        define_classic_palette!(@build [$($entries,)*] $($rest)*);
    };
    ($($commands:tt)*) => {
        define_classic_palette!(@build [] $($commands)*);
    };
}

macro_rules! define_interactive_commands {
    ($(
        $surface:ident(
            $kind:ident,
            $name:literal,
            [$($alias:literal),*],
            $usage:literal,
            $description:literal,
            $category:literal,
            $missing:literal
        );
    )*) => {
        pub(crate) const COMMAND_SPECS: &[InteractiveCommandSpec] = &[
            $(InteractiveCommandSpec {
                kind: InteractiveCommandKind::$kind,
                name: concat!("/", $name),
                bare_name: $name,
                aliases: &[$($alias),*],
                usage: $usage,
                description: $description,
                category: $category,
                surface: command_surface!($surface),
                missing_argument_usage: $missing,
            },)*
        ];

        define_classic_palette!($(
            $surface(
                $kind,
                $name,
                [$($alias),*],
                $usage,
                $description,
                $category,
                $missing
            );
        )*);
    };
}

define_interactive_commands! {
    shared(Help, "help", [], "/help", "Show the command reference", "Workspace", "");
    shared(Status, "status", [], "/status", "Show the active runtime and session", "Workspace", "");
    tui_only(Clear, "clear", [], "/clear", "Clear the visible conversation", "Workspace", "");
    tui_only(Mode, "mode", [], "/mode <build|plan>", "Switch between build and read-only planning", "Workspace", "");
    shared(Tools, "tools", [], "/tools", "Inspect connected tools and approval policy", "Runtime", "");
    tui_only(Shell, "shell", [], "/shell", "Toggle direct shell mode (`!command` also works)", "Workspace", "");
    shared(Capabilities, "capabilities", [], "/capabilities", "Browse work capability packs", "Runtime", "");
    shared(Essentials, "essentials", [], "/essentials", "Inspect prepackaged essential capabilities", "Runtime", "");
    shared(Mcp, "mcp", ["mcps"], "/mcp", "Inspect configured MCP servers and connected tools", "Runtime", "");
    shared(Skills, "skills", [], "/skills", "Browse and invoke discovered skills", "Runtime", "");
    shared(Plugins, "plugins", ["extensions"], "/plugins", "Inspect cross-CLI plugins and extensions", "Runtime", "");
    shared(Instructions, "instructions", ["context"], "/instructions [show]", "Inspect active AGENTS, Gemini, and Claude context", "Runtime", "");
    shared(Agents, "agents", [], "/agents", "Browse configured and specialist agents", "Runtime", "");
    shared(Teams, "teams", ["team"], "/teams [list|show NAME|validate [NAME]|topology NAME|run NAME TASK]", "Inspect and run governed ADK agent teams", "Work", "");
    shared(Inspect, "inspect", [], "/inspect", "Inspect the resolved runtime", "Runtime", "");
    shared(Doctor, "doctor", [], "/doctor", "Check MCP configuration readiness", "Runtime", "");
    shared(Usage, "usage", [], "/usage", "Show context-window utilization", "Session", "");
    shared(Sessions, "sessions", ["resume", "continue"], "/sessions [list|switch ID]", "List or switch persisted sessions", "Session", "");
    shared(NewSession, "new", [], "/new [session-id]", "Start a clean conversation session", "Session", "");
    tui_only(Copy, "copy", [], "/copy [all]", "Copy the last response to the clipboard (all = whole transcript)", "Session", "");
    tui_only(Mouse, "mouse", [], "/mouse [speed <1-20>]", "Trade the mouse wheel against native selection, or set the wheel step", "Session", "");
    tui_only(Export, "export", [], "/export [path.md]", "Export the visible transcript as Markdown", "Session", "");
    shared(Compact, "compact", [], "/compact", "Compact the active conversation", "Session", "");
    shared(Checkpoint, "checkpoint", [], "/checkpoint <save [label]|list|restore TAG>", "Save, inspect, or restore conversation state", "Session", "");
    shared(Tangent, "tangent", [], "/tangent [tail]", "Enter or leave an isolated conversation branch", "Session", "");
    shared(Undo, "undo", [], "/undo", "Undo the most recent tracked file edit", "Session", "");
    shared(Todos, "todos", [], "/todos [view ID|delete ID|clear-finished]", "Inspect and manage delegated task lists", "Work", "");
    shared(Delegate, "delegate", [], "/delegate [@agent|--agent NAME] <task>", "Run a named agent in an isolated retained session", "Work", "");
    shared(Parallel, "parallel", [], "/parallel @AGENT @AGENT <task>", "Run named agents concurrently with live child status", "Work", "/parallel @AGENT [@AGENT ...] <task>");
    shared(Spawn, "spawn", [], "/spawn @AGENT [--worktree] <task>", "Start a durable background agent", "Work", "/spawn @AGENT [--worktree] <task>");
    shared(Runs, "runs", [], "/runs", "Inspect durable background agent runs", "Work", "");
    shared(AgentSend, "send", [], "/send RUN_ID <message>", "Send a follow-up to a background agent", "Work", "/send RUN_ID <message>");
    shared(AgentCancel, "cancel", [], "/cancel RUN_ID", "Cancel a background agent run", "Work", "/cancel RUN_ID");
    shared(Ralph, "ralph", [], "/ralph <goal>", "Run the autonomous development pipeline", "Work", "");
    shared(Models, "models", [], "/models", "Show available model routes", "Models", "");
    shared(Provider, "provider", [], "/provider <provider>", "Switch the worker provider", "Models", "/provider <auto|gemini|openai|anthropic|deepseek|groq|ollama>");
    shared(Model, "model", [], "/model [model]", "Switch the worker model", "Models", "");
    shared(Worker, "worker", [], "/worker [model]", "Switch the worker model", "Models", "");
    shared(PlannerProvider, "planner-provider", [], "/planner-provider <provider>", "Switch the planner provider", "Models", "/planner-provider <openai|gemini|anthropic|deepseek|groq|ollama>");
    shared(Planner, "planner", [], "/planner [model]", "Switch the planner model", "Models", "");
    tui_only(Width, "width", [], "/width [full|comfortable|<columns>]", "Prose measure: fill the pane, or cap it for readability", "Settings", "");
    tui_only(Activity, "activity", [], "/activity [show|autohide|off]", "Run-history pane: pinned, revealed after a run, or hidden", "Settings", "");
    tui_only(Keys, "keys", [], "/keys", "List every keyboard shortcut this terminal can send", "Settings", "");
    shared(AutoCompact, "autocompact", [], "/autocompact", "Toggle automatic context compaction", "Settings", "");
    shared(Allow, "allow", [], "/allow <tool-pattern>", "Trust a tool pattern for this session", "Safety", "/allow <pattern> (e.g. 'execute_bash:git *', 'fs_read:*')");
    shared(Deny, "deny", [], "/deny <tool-pattern>", "Record a denied tool pattern", "Safety", "/deny <pattern> (e.g. 'execute_bash:rm -rf *', 'fs_write:/etc/*')");
    shared(Agent, "agent", [], "/agent", "Enable trusted agent mode after confirmation", "Safety", "");
    shared(Memory, "memory", [], "/memory <recall|remember|forget> <text>", "Use the durable memory service", "Utilities", "");
    shared(Time, "time", [], "/time [relative expression]", "Show or resolve date and time", "Utilities", "");
    shared(Exit, "exit", [], "/exit", "Close the workspace", "Workspace", "");
}

pub(crate) fn shared_command_specs() -> impl Iterator<Item = &'static InteractiveCommandSpec> {
    COMMAND_SPECS
        .iter()
        .filter(|spec| spec.surface == InteractiveCommandSurface::Shared)
}

pub(crate) fn find_shared_command(name: &str) -> Option<&'static InteractiveCommandSpec> {
    let name = name.trim_start_matches('/');
    shared_command_specs().find(|spec| {
        spec.bare_name.eq_ignore_ascii_case(name)
            || spec
                .aliases
                .iter()
                .any(|alias| alias.eq_ignore_ascii_case(name))
    })
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeSet;

    use super::*;
    use crate::chat::{ParsedChatCommand, format_chat_help, parse_chat_command};

    #[test]
    fn command_names_and_aliases_are_unique() {
        let mut names = BTreeSet::new();
        for spec in COMMAND_SPECS {
            assert!(
                COMMAND_CATEGORIES.contains(&spec.category),
                "unknown category {} for {}",
                spec.category,
                spec.name
            );
            assert!(
                names.insert(spec.bare_name),
                "duplicate command {}",
                spec.name
            );
            for alias in spec.aliases {
                assert!(names.insert(*alias), "duplicate command alias /{alias}");
            }
        }
    }

    #[test]
    fn every_shared_command_and_alias_is_recognized_by_the_parser() {
        for spec in shared_command_specs() {
            for name in std::iter::once(spec.bare_name).chain(spec.aliases.iter().copied()) {
                let parsed = parse_chat_command(&format!("/{name}"));
                assert!(
                    !matches!(parsed, ParsedChatCommand::UnknownCommand(_)),
                    "/{name} is catalogued but not parsed"
                );
            }
        }
    }

    #[test]
    fn generated_classic_help_includes_legacy_aliases_and_neutral_descriptions() {
        let help = format_chat_help();

        for spec in shared_command_specs() {
            assert!(
                help.contains(spec.usage),
                "{} is missing from help",
                spec.name
            );
            for alias in spec.aliases {
                assert!(
                    help.contains(&format!("/{alias}")),
                    "/{alias} is missing from help"
                );
            }
        }
        for alias in ["mcps", "extensions", "context", "resume", "continue"] {
            assert!(
                find_shared_command(alias).is_some(),
                "legacy alias /{alias} was removed"
            );
        }
        assert!(help.contains("Show the command reference"));
        assert!(help.contains("Show the active runtime and session"));
        assert!(!help.contains("keyboard actions"));
        assert!(!help.contains("session, and mode"));
    }

    #[test]
    fn tui_only_commands_are_not_exposed_by_classic_chat() {
        for spec in COMMAND_SPECS
            .iter()
            .filter(|spec| spec.surface == InteractiveCommandSurface::TuiOnly)
        {
            assert!(matches!(
                parse_chat_command(spec.name),
                ParsedChatCommand::UnknownCommand(_)
            ));
        }
    }
}
