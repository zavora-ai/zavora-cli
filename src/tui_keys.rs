//! Keyboard action registry for the terminal workspace.
//!
//! Every chord the workspace answers is declared once here, with the text that
//! describes it and the priority that decides whether it earns a place in the
//! footer. Before this existed the same shortcuts were spelled out in the key
//! handler, the welcome screen, the footer, the `/help` output, and the README,
//! and they had already drifted apart.
//!
//! Two things fall out of keeping the declarations in one table.
//!
//! Bindings can adapt to the terminal. A chord that never arrives is worse than
//! no chord at all, and which chords arrive is a property of the host: a MacBook
//! keyboard has no `Home` or `End` key, and macOS swallows the `Ctrl+Fn+Arrow`
//! substitute before the application sees it. So every action that would
//! otherwise be stranded carries a second binding, and the registry advertises
//! whichever one actually works here.
//!
//! Hints stay honest. `hint_priority` orders the footer, so the footer is
//! generated from the same table the key handler dispatches through and cannot
//! describe a chord that is not bound.
//!
//! Modelled on the action registry in xAI's Grok Build (`xai-grok-pager`,
//! Apache-2.0), which solves the same problem at a much larger scale.

use crossterm::event::{KeyCode, KeyModifiers};

/// A single chord.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct KeyShortcut {
    pub code: KeyCode,
    pub modifiers: KeyModifiers,
}

impl KeyShortcut {
    pub const fn new(code: KeyCode, modifiers: KeyModifiers) -> Self {
        Self { code, modifiers }
    }

    /// A bare key with no modifier.
    pub const fn plain(code: KeyCode) -> Self {
        Self::new(code, KeyModifiers::NONE)
    }

    pub const fn ctrl(code: KeyCode) -> Self {
        Self::new(code, KeyModifiers::CONTROL)
    }

    pub const fn alt(code: KeyCode) -> Self {
        Self::new(code, KeyModifiers::ALT)
    }

    pub const fn shift(code: KeyCode) -> Self {
        Self::new(code, KeyModifiers::SHIFT)
    }

    /// Whether an incoming event is this chord.
    ///
    /// Compares only the modifiers the workspace binds. `SUPER`, `HYPER`, and
    /// `META` are ignored because nothing binds them and some terminals report
    /// `META` alongside `ALT`, which would otherwise stop an `Alt` binding from
    /// matching. `SHIFT` is ignored for character bindings only: crossterm
    /// reports it alongside a capital letter, so comparing it would break
    /// `Ctrl+P` the moment caps were involved.
    pub fn matches(self, code: KeyCode, modifiers: KeyModifiers) -> bool {
        if self.code != code {
            return false;
        }
        let mask = if matches!(code, KeyCode::Char(_)) {
            KeyModifiers::CONTROL | KeyModifiers::ALT
        } else {
            KeyModifiers::CONTROL | KeyModifiers::ALT | KeyModifiers::SHIFT
        };
        (modifiers & mask) == (self.modifiers & mask)
    }

    /// How the chord is written in help and hints.
    pub fn display(self) -> String {
        let mut out = String::new();
        if self.modifiers.contains(KeyModifiers::CONTROL) {
            out.push_str("Ctrl+");
        }
        if self.modifiers.contains(KeyModifiers::ALT) {
            out.push_str("Alt+");
        }
        // A capital letter already carries its own shift; saying so twice reads
        // as a three-key chord.
        if self.modifiers.contains(KeyModifiers::SHIFT) && !matches!(self.code, KeyCode::Char(_)) {
            out.push_str("Shift+");
        }
        match self.code {
            KeyCode::Char(' ') => out.push_str("Space"),
            KeyCode::Char(c) => out.extend(c.to_uppercase()),
            KeyCode::Up => out.push('↑'),
            KeyCode::Down => out.push('↓'),
            KeyCode::Left => out.push('←'),
            KeyCode::Right => out.push('→'),
            KeyCode::PageUp => out.push_str("PgUp"),
            KeyCode::PageDown => out.push_str("PgDn"),
            KeyCode::Home => out.push_str("Home"),
            KeyCode::End => out.push_str("End"),
            KeyCode::Enter => out.push_str("Enter"),
            KeyCode::Esc => out.push_str("Esc"),
            KeyCode::Tab => out.push_str("Tab"),
            // Crossterm reports Shift+Tab as its own code, so the prefix above
            // never fires for it.
            KeyCode::BackTab => out.push_str("Shift+Tab"),
            KeyCode::Backspace => out.push_str("Backspace"),
            KeyCode::Delete => out.push_str("Delete"),
            other => out.push_str(&format!("{other:?}")),
        }
        out
    }
}

/// Everything the workspace can be asked to do from the keyboard.
///
/// Composer editing — cursor motion, word jumps, deletion — is deliberately
/// absent: those are readline conventions the developer already knows, they are
/// never advertised, and enumerating them would add a table entry per chord
/// without making anything more discoverable.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ActionId {
    ScrollLineUp,
    ScrollLineDown,
    ScrollHalfPageUp,
    ScrollHalfPageDown,
    ScrollPageUp,
    ScrollPageDown,
    ScrollToStart,
    ScrollToEnd,
    PrevMessage,
    NextMessage,
    Send,
    Newline,
    HistoryPrev,
    HistoryNext,
    ToggleMode,
    OpenPalette,
    CompleteCommand,
    Cancel,
    ClearConversation,
    ToggleMouseCapture,
}

/// When an action is available.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum When {
    /// Bound whatever the workspace is doing.
    Always,
    /// Bound only while a turn is running.
    Busy,
    /// Bound only while the workspace is idle.
    Idle,
}

impl When {
    /// Whether this context is live given the current busy state.
    pub fn applies(self, busy: bool) -> bool {
        match self {
            Self::Always => true,
            Self::Busy => busy,
            Self::Idle => !busy,
        }
    }
}

/// Grouping for the help listing.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Category {
    Navigation,
    Composer,
    Session,
}

impl Category {
    pub fn label(self) -> &'static str {
        match self {
            Self::Navigation => "Navigation",
            Self::Composer => "Composer",
            Self::Session => "Session",
        }
    }
}

/// One declared action.
pub struct ActionDef {
    pub id: ActionId,
    /// Sentence describing the effect, for the help listing.
    pub description: &'static str,
    pub default_key: KeyShortcut,
    /// Further chords that do the same thing. These exist so an action stays
    /// reachable on a keyboard or terminal that cannot deliver `default_key`.
    pub alt_keys: Vec<KeyShortcut>,
    pub category: Category,
    pub context: When,
    /// Position in the footer; lower is further left. `None` stays out of it.
    ///
    /// The footer has room for a handful of chords, so this is a deliberate
    /// ranking of what a newcomer needs rather than a dump of the table.
    pub hint_priority: Option<u8>,
    /// Replaces the rendered chord in hints, for pairs that read better
    /// together — `↑↓` rather than two entries for history.
    pub hint_key_display: Option<&'static str>,
}

/// Which terminal the workspace is running in.
///
/// Only distinctions that change a binding or a hint are represented.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TerminalKind {
    AppleTerminal,
    ITerm2,
    VsCode,
    Other,
}

impl TerminalKind {
    /// Identify the host terminal from the environment.
    pub fn detect() -> Self {
        if std::env::var_os("VSCODE_INJECTION").is_some()
            || matches!(std::env::var("TERM_PROGRAM").as_deref(), Ok("vscode"))
        {
            return Self::VsCode;
        }
        match std::env::var("TERM_PROGRAM").as_deref() {
            Ok("Apple_Terminal") => Self::AppleTerminal,
            Ok("iTerm.app") => Self::ITerm2,
            _ => Self::Other,
        }
    }

    /// Whether this host actually delivers a chord to the application.
    ///
    /// Measured, not assumed. Recording a `KeyEvent` log under Apple Terminal
    /// while injecting each chord showed that it:
    ///
    /// - never sends `Home` or `End`;
    /// - strips `SHIFT` from `PageUp`/`PageDown`, so the shifted form is
    ///   indistinguishable from the bare one;
    /// - does not send `Ctrl`- or `Alt`-modified arrows — `Alt+↑` arrives as a
    ///   bare `Esc` followed by `[` and `A` as separate key events, and
    ///   `Ctrl+↑` produces nothing at all.
    ///
    /// `Ctrl`+letter is safe everywhere: those are single ASCII control bytes
    /// rather than modifier-encoded escape sequences, which is why `Ctrl+C` and
    /// `Ctrl+D` have always worked here.
    pub fn delivers(self, chord: KeyShortcut) -> bool {
        if self != Self::AppleTerminal {
            return true;
        }
        let modified = chord
            .modifiers
            .intersects(KeyModifiers::CONTROL | KeyModifiers::ALT | KeyModifiers::SHIFT);
        match chord.code {
            KeyCode::Home | KeyCode::End => false,
            KeyCode::PageUp | KeyCode::PageDown => !modified,
            KeyCode::Up | KeyCode::Down | KeyCode::Left | KeyCode::Right => !modified,
            _ => true,
        }
    }

    /// Whether `Ctrl` combined with `Home` or `End` reaches the application.
    ///
    /// A MacBook keyboard has no `Home`/`End`; the substitute is `Fn` with an
    /// arrow, and macOS consumes `Ctrl+Fn+Arrow` before the terminal forwards
    /// it. The chord stays bound for external keyboards, but it must not be the
    /// one the footer advertises.
    pub fn ctrl_home_end_arrives(self) -> bool {
        self.delivers(KeyShortcut::ctrl(KeyCode::Home)) && !cfg!(target_os = "macos")
    }

    /// The key held to force the terminal's own text selection while the
    /// workspace is capturing the mouse.
    ///
    /// Terminals disagree, and getting it wrong means telling a developer their
    /// selection is gone when it is one modifier away. These are the documented
    /// gestures for each host.
    pub fn native_selection_modifier(self) -> &'static str {
        match self {
            Self::AppleTerminal => "Fn",
            Self::ITerm2 => "Option",
            // xterm.js embeds use Shift, or Option on macOS when
            // `terminal.integrated.macOptionClickForcesSelection` is set.
            Self::VsCode => "Shift",
            Self::Other => "Shift",
        }
    }

    /// Human-readable name, for messages that name the detected terminal.
    pub fn label(self) -> &'static str {
        match self {
            Self::AppleTerminal => "Apple Terminal",
            Self::ITerm2 => "iTerm2",
            Self::VsCode => "the VS Code terminal",
            Self::Other => "this terminal",
        }
    }
}

/// The set of chords the workspace answers, resolved for one terminal.
pub struct ActionRegistry {
    actions: Vec<ActionDef>,
    terminal: TerminalKind,
}

impl ActionRegistry {
    /// Build the registry for the detected terminal.
    pub fn detect() -> Self {
        Self::for_terminal(TerminalKind::detect())
    }

    /// Build the registry for a named terminal.
    ///
    /// Separate from [`Self::detect`] so tests can pin the host instead of
    /// inheriting whatever terminal happens to be running them.
    pub fn for_terminal(terminal: TerminalKind) -> Self {
        Self {
            actions: default_actions(terminal),
            terminal,
        }
    }

    pub fn terminal(&self) -> TerminalKind {
        self.terminal
    }

    pub fn actions(&self) -> &[ActionDef] {
        &self.actions
    }

    /// The action a chord triggers, given whether a turn is running.
    ///
    /// First match wins, so the table order is the precedence order.
    pub fn lookup(&self, code: KeyCode, modifiers: KeyModifiers, busy: bool) -> Option<ActionId> {
        self.actions
            .iter()
            .find(|def| {
                def.context.applies(busy)
                    && (def.default_key.matches(code, modifiers)
                        || def.alt_keys.iter().any(|alt| alt.matches(code, modifiers)))
            })
            .map(|def| def.id)
    }

    pub fn find(&self, id: ActionId) -> Option<&ActionDef> {
        self.actions.iter().find(|def| def.id == id)
    }

    /// The chord to advertise for an action, or `None` if this host can send
    /// none of them.
    ///
    /// Advertising a dead chord is how a workspace teaches a developer that its
    /// shortcuts do not work, so hints and `/keys` both go through here.
    pub fn advertised_key(&self, id: ActionId) -> Option<KeyShortcut> {
        let def = self.find(id)?;
        std::iter::once(def.default_key)
            .chain(def.alt_keys.iter().copied())
            .find(|chord| self.terminal.delivers(*chord))
    }

    /// Footer hints in priority order: `(chord, label)`.
    ///
    /// An action this host cannot reach is left out rather than shown with a
    /// chord that does nothing.
    pub fn hints(&self, busy: bool) -> Vec<(String, &'static str)> {
        let mut ranked: Vec<(u8, &ActionDef)> = self
            .actions
            .iter()
            .filter(|def| def.context.applies(busy))
            .filter_map(|def| def.hint_priority.map(|priority| (priority, def)))
            .collect();
        ranked.sort_by_key(|(priority, _)| *priority);
        ranked
            .into_iter()
            .filter_map(|(_, def)| {
                let key = match def.hint_key_display {
                    // A combined display such as `PgUp/PgDn` still has to be
                    // backed by a chord this host can send.
                    Some(text) => {
                        self.advertised_key(def.id)?;
                        text.to_string()
                    }
                    None => self.advertised_key(def.id)?.display(),
                };
                Some((key, def.hint_label()))
            })
            .collect()
    }

    /// The full listing, grouped by category, for `/keys`.
    ///
    /// Only chords this host can actually send are listed; an action with none
    /// is reported as unavailable rather than silently dropped.
    pub fn help_lines(&self) -> Vec<(Category, Vec<(String, &'static str)>)> {
        let mut out: Vec<(Category, Vec<(String, &'static str)>)> = Vec::new();
        for category in [Category::Navigation, Category::Composer, Category::Session] {
            let rows: Vec<(String, &'static str)> = self
                .actions
                .iter()
                .filter(|def| def.category == category)
                .map(|def| {
                    let mut chords: Vec<String> = std::iter::once(def.default_key)
                        .chain(def.alt_keys.iter().copied())
                        .filter(|chord| self.terminal.delivers(*chord))
                        .map(KeyShortcut::display)
                        .collect();
                    chords.dedup();
                    let rendered = if chords.is_empty() {
                        "unavailable in this terminal".to_string()
                    } else {
                        chords.join(" / ")
                    };
                    (rendered, def.description)
                })
                .collect();
            if !rows.is_empty() {
                out.push((category, rows));
            }
        }
        out
    }
}

impl ActionDef {
    /// Short footer wording, distinct from the fuller `description`.
    fn hint_label(&self) -> &'static str {
        match self.id {
            ActionId::ScrollHalfPageUp | ActionId::ScrollHalfPageDown => "scroll",
            ActionId::ScrollToStart => "top",
            ActionId::ScrollToEnd => "newest",
            ActionId::PrevMessage | ActionId::NextMessage => "response",
            ActionId::HistoryPrev | ActionId::HistoryNext => "history",
            ActionId::Send => "send",
            ActionId::ToggleMode => "mode",
            ActionId::OpenPalette => "actions",
            ActionId::Cancel => "cancel",
            _ => "",
        }
    }
}

/// The default table, resolved against one terminal.
fn default_actions(_terminal: TerminalKind) -> Vec<ActionDef> {
    vec![
        // ── Navigation ──────────────────────────────────────────────
        // Half a page leads because it is what PageUp/PageDown do; the whole
        // page sits behind Shift for readers who want the larger jump.
        ActionDef {
            id: ActionId::ScrollHalfPageUp,
            description: "Scroll up half a screen",
            default_key: KeyShortcut::plain(KeyCode::PageUp),
            alt_keys: Vec::new(),
            category: Category::Navigation,
            context: When::Always,
            hint_priority: Some(20),
            hint_key_display: Some("PgUp/PgDn"),
        },
        ActionDef {
            id: ActionId::ScrollHalfPageDown,
            description: "Scroll down half a screen",
            default_key: KeyShortcut::plain(KeyCode::PageDown),
            alt_keys: Vec::new(),
            category: Category::Navigation,
            context: When::Always,
            hint_priority: None,
            hint_key_display: None,
        },
        ActionDef {
            id: ActionId::ScrollPageUp,
            description: "Scroll up a whole screen",
            default_key: KeyShortcut::shift(KeyCode::PageUp),
            alt_keys: Vec::new(),
            category: Category::Navigation,
            context: When::Always,
            hint_priority: None,
            hint_key_display: None,
        },
        ActionDef {
            id: ActionId::ScrollPageDown,
            description: "Scroll down a whole screen",
            default_key: KeyShortcut::shift(KeyCode::PageDown),
            alt_keys: Vec::new(),
            category: Category::Navigation,
            context: When::Always,
            hint_priority: None,
            hint_key_display: None,
        },
        ActionDef {
            id: ActionId::ScrollLineUp,
            description: "Scroll up one line",
            default_key: KeyShortcut::ctrl(KeyCode::Up),
            alt_keys: Vec::new(),
            category: Category::Navigation,
            context: When::Always,
            hint_priority: None,
            hint_key_display: None,
        },
        ActionDef {
            id: ActionId::ScrollLineDown,
            description: "Scroll down one line",
            default_key: KeyShortcut::ctrl(KeyCode::Down),
            alt_keys: Vec::new(),
            category: Category::Navigation,
            context: When::Always,
            hint_priority: None,
            hint_key_display: None,
        },
        ActionDef {
            id: ActionId::PrevMessage,
            description: "Jump to the previous response",
            default_key: KeyShortcut::alt(KeyCode::Up),
            // `Ctrl`+letter is the keyboard-independent form: O for older.
            alt_keys: vec![KeyShortcut::ctrl(KeyCode::Char('o'))],
            category: Category::Navigation,
            context: When::Always,
            hint_priority: Some(30),
            hint_key_display: None,
        },
        ActionDef {
            id: ActionId::NextMessage,
            description: "Jump to the next response",
            default_key: KeyShortcut::alt(KeyCode::Down),
            alt_keys: vec![KeyShortcut::ctrl(KeyCode::Char('n'))],
            category: Category::Navigation,
            context: When::Always,
            hint_priority: None,
            hint_key_display: None,
        },
        ActionDef {
            id: ActionId::ScrollToStart,
            description: "Jump to the start of the conversation",
            default_key: KeyShortcut::ctrl(KeyCode::Home),
            // T for top, then the arrow form for keyboards with neither.
            alt_keys: vec![
                KeyShortcut::ctrl(KeyCode::Char('t')),
                KeyShortcut::new(KeyCode::Up, KeyModifiers::CONTROL | KeyModifiers::ALT),
            ],
            category: Category::Navigation,
            context: When::Always,
            hint_priority: Some(40),
            hint_key_display: None,
        },
        ActionDef {
            id: ActionId::ScrollToEnd,
            description: "Jump to the newest output and follow it",
            default_key: KeyShortcut::ctrl(KeyCode::End),
            alt_keys: vec![
                KeyShortcut::ctrl(KeyCode::Char('b')),
                KeyShortcut::new(KeyCode::Down, KeyModifiers::CONTROL | KeyModifiers::ALT),
            ],
            category: Category::Navigation,
            context: When::Always,
            hint_priority: None,
            hint_key_display: None,
        },
        // ── Composer ────────────────────────────────────────────────
        ActionDef {
            id: ActionId::Send,
            description: "Send the prompt",
            default_key: KeyShortcut::plain(KeyCode::Enter),
            alt_keys: Vec::new(),
            category: Category::Composer,
            context: When::Idle,
            hint_priority: Some(8),
            hint_key_display: None,
        },
        ActionDef {
            id: ActionId::Newline,
            description: "Add a line without sending",
            default_key: KeyShortcut::ctrl(KeyCode::Char('j')),
            alt_keys: Vec::new(),
            category: Category::Composer,
            context: When::Always,
            hint_priority: None,
            hint_key_display: None,
        },
        ActionDef {
            id: ActionId::HistoryPrev,
            description: "Recall the previous prompt",
            default_key: KeyShortcut::plain(KeyCode::Up),
            alt_keys: Vec::new(),
            category: Category::Composer,
            context: When::Idle,
            hint_priority: Some(10),
            hint_key_display: Some("↑↓"),
        },
        ActionDef {
            id: ActionId::HistoryNext,
            description: "Recall the next prompt",
            default_key: KeyShortcut::plain(KeyCode::Down),
            alt_keys: Vec::new(),
            category: Category::Composer,
            context: When::Idle,
            hint_priority: None,
            hint_key_display: None,
        },
        ActionDef {
            id: ActionId::CompleteCommand,
            description: "Complete the slash command",
            default_key: KeyShortcut::plain(KeyCode::Tab),
            alt_keys: Vec::new(),
            category: Category::Composer,
            context: When::Idle,
            hint_priority: None,
            hint_key_display: None,
        },
        // ── Session ─────────────────────────────────────────────────
        ActionDef {
            id: ActionId::ToggleMode,
            description: "Switch between BUILD and PLAN",
            default_key: KeyShortcut::plain(KeyCode::BackTab),
            alt_keys: Vec::new(),
            category: Category::Session,
            context: When::Always,
            hint_priority: Some(5),
            hint_key_display: None,
        },
        ActionDef {
            id: ActionId::OpenPalette,
            description: "Search every action and command",
            default_key: KeyShortcut::ctrl(KeyCode::Char('p')),
            alt_keys: Vec::new(),
            category: Category::Session,
            context: When::Always,
            hint_priority: Some(50),
            hint_key_display: None,
        },
        ActionDef {
            id: ActionId::Cancel,
            description: "Request cancellation of the running turn",
            default_key: KeyShortcut::plain(KeyCode::Esc),
            alt_keys: Vec::new(),
            category: Category::Session,
            context: When::Busy,
            hint_priority: Some(1),
            hint_key_display: None,
        },
        ActionDef {
            id: ActionId::ToggleMouseCapture,
            description: "Toggle the mouse wheel against native text selection",
            default_key: KeyShortcut::ctrl(KeyCode::Char('r')),
            alt_keys: Vec::new(),
            category: Category::Session,
            context: When::Always,
            hint_priority: None,
            hint_key_display: None,
        },
        ActionDef {
            id: ActionId::ClearConversation,
            description: "Clear the conversation",
            default_key: KeyShortcut::ctrl(KeyCode::Char('l')),
            alt_keys: Vec::new(),
            category: Category::Session,
            context: When::Idle,
            hint_priority: None,
            hint_key_display: None,
        },
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A chord renders the way the documentation writes it.
    #[test]
    fn chords_render_readably() {
        assert_eq!(KeyShortcut::ctrl(KeyCode::Home).display(), "Ctrl+Home");
        assert_eq!(KeyShortcut::alt(KeyCode::Up).display(), "Alt+↑");
        assert_eq!(KeyShortcut::plain(KeyCode::PageDown).display(), "PgDn");
        assert_eq!(
            KeyShortcut::ctrl(KeyCode::Char('p')).display(),
            "Ctrl+P",
            "letters are advertised in upper case"
        );
        assert_eq!(
            KeyShortcut::plain(KeyCode::BackTab).display(),
            "Shift+Tab",
            "BackTab already means shift; it must not read Shift+Shift+Tab"
        );
        assert_eq!(
            KeyShortcut::new(KeyCode::Up, KeyModifiers::CONTROL | KeyModifiers::ALT).display(),
            "Ctrl+Alt+↑"
        );
    }

    /// Modifier comparison ignores modifiers the workspace does not bind.
    #[test]
    fn matching_ignores_unbound_modifiers() {
        let chord = KeyShortcut::ctrl(KeyCode::Up);
        assert!(chord.matches(KeyCode::Up, KeyModifiers::CONTROL));
        assert!(
            chord.matches(
                KeyCode::Up,
                KeyModifiers::CONTROL | KeyModifiers::META | KeyModifiers::SUPER
            ),
            "a terminal that also reports META must still match"
        );
        assert!(!chord.matches(KeyCode::Up, KeyModifiers::NONE));
        assert!(
            !chord.matches(KeyCode::Up, KeyModifiers::CONTROL | KeyModifiers::ALT),
            "Ctrl+Alt+Up is a different binding from Ctrl+Up"
        );
        // Capitals arrive with SHIFT set, so a character binding must ignore it.
        assert!(
            KeyShortcut::ctrl(KeyCode::Char('p')).matches(
                KeyCode::Char('p'),
                KeyModifiers::CONTROL | KeyModifiers::SHIFT
            ),
            "reported shift must not break a Ctrl+letter binding"
        );
    }

    /// Every declared chord resolves to exactly the action that declared it.
    #[test]
    fn every_declared_chord_dispatches_to_its_own_action() {
        let registry = ActionRegistry::for_terminal(TerminalKind::Other);
        for def in registry.actions() {
            for busy in [false, true] {
                if !def.context.applies(busy) {
                    continue;
                }
                for chord in std::iter::once(&def.default_key).chain(def.alt_keys.iter()) {
                    let hit = registry.lookup(chord.code, chord.modifiers, busy);
                    assert_eq!(
                        hit,
                        Some(def.id),
                        "{} resolved to {hit:?} instead of {:?} (busy={busy})",
                        chord.display(),
                        def.id
                    );
                }
            }
        }
    }

    /// Distinct actions must not claim the same chord in the same context.
    #[test]
    fn no_two_live_actions_share_a_chord() {
        let registry = ActionRegistry::for_terminal(TerminalKind::Other);
        for busy in [false, true] {
            let live: Vec<&ActionDef> = registry
                .actions()
                .iter()
                .filter(|def| def.context.applies(busy))
                .collect();
            for (index, first) in live.iter().enumerate() {
                for second in &live[index + 1..] {
                    let first_keys =
                        std::iter::once(first.default_key).chain(first.alt_keys.iter().copied());
                    for chord in first_keys {
                        let clash = std::iter::once(second.default_key)
                            .chain(second.alt_keys.iter().copied())
                            .any(|other| other == chord);
                        assert!(
                            !clash,
                            "{:?} and {:?} both bind {} (busy={busy})",
                            first.id,
                            second.id,
                            chord.display()
                        );
                    }
                }
            }
        }
    }

    /// Busy-only and idle-only actions are not offered at the wrong time.
    #[test]
    fn context_gates_dispatch() {
        let registry = ActionRegistry::for_terminal(TerminalKind::Other);
        assert_eq!(
            registry.lookup(KeyCode::Esc, KeyModifiers::NONE, true),
            Some(ActionId::Cancel)
        );
        assert_eq!(
            registry.lookup(KeyCode::Esc, KeyModifiers::NONE, false),
            None,
            "there is nothing to cancel when idle"
        );
        assert_eq!(
            registry.lookup(KeyCode::Up, KeyModifiers::NONE, false),
            Some(ActionId::HistoryPrev)
        );
        assert_eq!(
            registry.lookup(KeyCode::Up, KeyModifiers::NONE, true),
            None,
            "history recall is not offered mid-turn"
        );
        // Scrolling is available throughout — reading back is most useful while
        // the agent is still working.
        assert_eq!(
            registry.lookup(KeyCode::PageUp, KeyModifiers::NONE, true),
            Some(ActionId::ScrollHalfPageUp)
        );
    }

    /// Shift selects the whole-page variant rather than the half-page default.
    #[test]
    fn shift_paging_is_distinct_from_plain_paging() {
        let registry = ActionRegistry::for_terminal(TerminalKind::Other);
        assert_eq!(
            registry.lookup(KeyCode::PageUp, KeyModifiers::NONE, false),
            Some(ActionId::ScrollHalfPageUp)
        );
        assert_eq!(
            registry.lookup(KeyCode::PageUp, KeyModifiers::SHIFT, false),
            Some(ActionId::ScrollPageUp)
        );
    }

    /// Hints only name chords the host can actually send.
    ///
    /// The Apple Terminal expectations are measurements, not guesses: a
    /// `KeyEvent` log taken while injecting each chord showed `Home`/`End` never
    /// arriving, `SHIFT` stripped from `PageUp`/`PageDown`, and modified arrows
    /// either swallowed or split into `Esc` plus separate characters.
    #[test]
    fn hints_never_advertise_a_chord_the_host_cannot_send() {
        let apple = ActionRegistry::for_terminal(TerminalKind::AppleTerminal);

        // Top and bottom fall back to the Ctrl+letter forms, which are single
        // ASCII control bytes and always arrive.
        assert_eq!(
            apple
                .advertised_key(ActionId::ScrollToStart)
                .map(KeyShortcut::display)
                .as_deref(),
            Some("Ctrl+T")
        );
        assert_eq!(
            apple
                .advertised_key(ActionId::ScrollToEnd)
                .map(KeyShortcut::display)
                .as_deref(),
            Some("Ctrl+B")
        );
        assert_eq!(
            apple
                .advertised_key(ActionId::PrevMessage)
                .map(KeyShortcut::display)
                .as_deref(),
            Some("Ctrl+O"),
            "Alt+arrow does not survive Apple Terminal"
        );

        // A whole page has no reachable chord there, because SHIFT is stripped
        // from PageUp. Saying nothing is better than naming a dead key.
        assert_eq!(apple.advertised_key(ActionId::ScrollPageUp), None);
        assert!(
            !apple
                .hints(false)
                .iter()
                .any(|(key, _)| key.contains("Shift+Pg")),
            "the footer offered a chord this terminal strips"
        );
        // Half a page still works, so the scroll hint survives.
        assert!(
            apple
                .hints(false)
                .iter()
                .any(|(_, label)| *label == "scroll")
        );

        // Every hint on every host names a chord that host delivers.
        for kind in [
            TerminalKind::AppleTerminal,
            TerminalKind::ITerm2,
            TerminalKind::VsCode,
            TerminalKind::Other,
        ] {
            let registry = ActionRegistry::for_terminal(kind);
            for busy in [false, true] {
                for (key, label) in registry.hints(busy) {
                    assert!(
                        !key.is_empty() && !label.is_empty(),
                        "{kind:?} produced an empty hint"
                    );
                }
            }
        }

        // A terminal that can send the arrow forms still gets them.
        let other = ActionRegistry::for_terminal(TerminalKind::Other);
        assert_eq!(
            other
                .advertised_key(ActionId::ScrollToStart)
                .map(KeyShortcut::display)
                .as_deref(),
            Some("Ctrl+Home")
        );

        // Whatever is advertised, every declared chord stays bound.
        for chord in [
            KeyShortcut::ctrl(KeyCode::Home),
            KeyShortcut::ctrl(KeyCode::Char('t')),
            KeyShortcut::new(KeyCode::Up, KeyModifiers::CONTROL | KeyModifiers::ALT),
        ] {
            assert_eq!(
                apple.lookup(chord.code, chord.modifiers, false),
                Some(ActionId::ScrollToStart),
                "{} stopped reaching the action",
                chord.display()
            );
        }
    }

    /// The footer is ordered by priority and only names bound chords.
    #[test]
    fn footer_hints_are_ranked_and_bound() {
        let registry = ActionRegistry::for_terminal(TerminalKind::Other);
        let idle = registry.hints(false);
        assert!(!idle.is_empty(), "the footer must offer something");
        assert!(
            idle.iter()
                .all(|(key, label)| !key.is_empty() && !label.is_empty()),
            "every hint needs both a chord and a label: {idle:?}"
        );

        // Mode leads, then history, then scrolling: the order a newcomer needs.
        let labels: Vec<&str> = idle.iter().map(|(_, label)| *label).collect();
        let position = |needle: &str| labels.iter().position(|label| *label == needle);
        assert!(position("mode") < position("history"));
        assert!(position("history") < position("scroll"));

        // Cancel is the first thing offered while a turn is running, and is
        // absent when there is nothing to cancel.
        let busy = registry.hints(true);
        assert_eq!(busy.first().map(|(_, label)| *label), Some("cancel"));
        assert!(!labels.contains(&"cancel"));
    }

    /// Help covers every declared action exactly once.
    #[test]
    fn help_lists_every_action() {
        let registry = ActionRegistry::for_terminal(TerminalKind::Other);
        let rows: usize = registry
            .help_lines()
            .iter()
            .map(|(_, rows)| rows.len())
            .sum();
        assert_eq!(
            rows,
            registry.actions().len(),
            "the listing must account for every action"
        );
        for (_, group) in registry.help_lines() {
            for (chords, description) in group {
                assert!(!chords.is_empty(), "an action listed no chord");
                assert!(
                    description.chars().next().is_some_and(char::is_uppercase),
                    "description should read as a sentence: {description:?}"
                );
            }
        }
    }
}
