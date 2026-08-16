//! Full-screen terminal workspace for interactive Zavora sessions.

use std::io;
use std::sync::Arc;
use std::time::{Duration, Instant};

use adk_rust::prelude::{Content, Event, Runner};
use anyhow::{Context, Result};
use crossterm::event::{
    self, DisableMouseCapture, EnableBracketedPaste, EnableMouseCapture, Event as TerminalEvent,
    KeyCode, KeyEvent, KeyEventKind, KeyModifiers, MouseButton, MouseEventKind,
};
use crossterm::execute;
use crossterm::terminal::{EnterAlternateScreen, disable_raw_mode, enable_raw_mode};
use ratatui::Frame;
use ratatui::backend::CrosstermBackend;
use ratatui::layout::{Alignment, Constraint, Direction, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span, Text};
use ratatui::widgets::{Block, Borders, Clear, List, ListItem, Paragraph, Wrap};
use unicode_width::{UnicodeWidthChar, UnicodeWidthStr};

use crate::chat::{ChatCommand, ParsedChatCommand, parse_chat_command};
use crate::checkpoint::{
    CheckpointStore, format_checkpoint_list, restore_session_events, snapshot_session_events,
};
use crate::config::RuntimeConfig;
use crate::context::compute_context_usage;
use crate::guardrail::{apply_guardrail, buffered_output_required, enforce_prompt_limit};
use crate::retrieval::{RetrievalPolicy, RetrievalService, augment_prompt_with_retrieval};
use crate::runner::{ResolvedRuntimeTools, ToolConfirmationSettings, build_single_runner_for_chat};
use crate::session::build_session_service;
use crate::streaming::{UiEvent, run_prompt, run_prompt_to_ui};
use crate::telemetry::TelemetrySink;
use crate::tools::confirming::{ApprovalDecision, clear_approval_bridge, install_approval_bridge};

const ORANGE: Color = Color::Rgb(255, 105, 70);
const PANEL: Color = Color::Reset;
const MUTED: Color = Color::DarkGray;
const TEXT: Color = Color::Reset;
const CODE_TEXT: Color = Color::Rgb(226, 229, 235);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct CommandSpec {
    name: &'static str,
    usage: &'static str,
    description: &'static str,
    category: &'static str,
}

const COMMAND_SPECS: &[CommandSpec] = &[
    CommandSpec {
        name: "/help",
        usage: "/help",
        description: "Search commands and keyboard actions",
        category: "Workspace",
    },
    CommandSpec {
        name: "/status",
        usage: "/status",
        description: "Show the active profile, routes, session, and mode",
        category: "Workspace",
    },
    CommandSpec {
        name: "/clear",
        usage: "/clear",
        description: "Clear the visible conversation",
        category: "Workspace",
    },
    CommandSpec {
        name: "/mode",
        usage: "/mode <build|plan>",
        description: "Switch between build and read-only planning",
        category: "Workspace",
    },
    CommandSpec {
        name: "/tools",
        usage: "/tools",
        description: "Inspect connected tools and approval policy",
        category: "Runtime",
    },
    CommandSpec {
        name: "/shell",
        usage: "/shell",
        description: "Toggle direct shell mode (`!command` also works)",
        category: "Workspace",
    },
    CommandSpec {
        name: "/capabilities",
        usage: "/capabilities",
        description: "Browse work capability packs",
        category: "Runtime",
    },
    CommandSpec {
        name: "/mcp",
        usage: "/mcp",
        description: "Inspect configured MCP servers and connected tools",
        category: "Runtime",
    },
    CommandSpec {
        name: "/skills",
        usage: "/skills",
        description: "Browse and invoke discovered skills",
        category: "Runtime",
    },
    CommandSpec {
        name: "/plugins",
        usage: "/plugins",
        description: "Inspect cross-CLI plugins and extensions",
        category: "Runtime",
    },
    CommandSpec {
        name: "/instructions",
        usage: "/instructions [show]",
        description: "Inspect active AGENTS, Gemini, and Claude context",
        category: "Runtime",
    },
    CommandSpec {
        name: "/agents",
        usage: "/agents",
        description: "Browse configured and specialist agents",
        category: "Runtime",
    },
    CommandSpec {
        name: "/inspect",
        usage: "/inspect",
        description: "Inspect the resolved runtime",
        category: "Runtime",
    },
    CommandSpec {
        name: "/doctor",
        usage: "/doctor",
        description: "Check MCP configuration readiness",
        category: "Runtime",
    },
    CommandSpec {
        name: "/usage",
        usage: "/usage",
        description: "Show context-window utilization",
        category: "Session",
    },
    CommandSpec {
        name: "/sessions",
        usage: "/sessions [list|switch ID]",
        description: "List or switch persisted sessions",
        category: "Session",
    },
    CommandSpec {
        name: "/new",
        usage: "/new [session-id]",
        description: "Start a clean conversation session",
        category: "Session",
    },
    CommandSpec {
        name: "/copy",
        usage: "/copy [all]",
        description: "Copy the last response to the clipboard (all = whole transcript)",
        category: "Session",
    },
    CommandSpec {
        name: "/mouse",
        usage: "/mouse [speed <1-20>]",
        description: "Trade the mouse wheel against native selection, or set the wheel step",
        category: "Session",
    },
    CommandSpec {
        name: "/export",
        usage: "/export [path.md]",
        description: "Export the visible transcript as Markdown",
        category: "Session",
    },
    CommandSpec {
        name: "/compact",
        usage: "/compact",
        description: "Compact the active conversation",
        category: "Session",
    },
    CommandSpec {
        name: "/checkpoint",
        usage: "/checkpoint <save [label]|list|restore TAG>",
        description: "Save, inspect, or restore conversation state",
        category: "Session",
    },
    CommandSpec {
        name: "/tangent",
        usage: "/tangent [tail]",
        description: "Enter or leave an isolated conversation branch",
        category: "Session",
    },
    CommandSpec {
        name: "/undo",
        usage: "/undo",
        description: "Undo the most recent tracked file edit",
        category: "Session",
    },
    CommandSpec {
        name: "/todos",
        usage: "/todos [view ID|delete ID|clear-finished]",
        description: "Inspect and manage delegated task lists",
        category: "Work",
    },
    CommandSpec {
        name: "/delegate",
        usage: "/delegate <task>",
        description: "Run an isolated subagent task",
        category: "Work",
    },
    CommandSpec {
        name: "/ralph",
        usage: "/ralph <goal>",
        description: "Run the autonomous development pipeline",
        category: "Work",
    },
    CommandSpec {
        name: "/models",
        usage: "/models",
        description: "Show available model routes",
        category: "Models",
    },
    CommandSpec {
        name: "/provider",
        usage: "/provider <provider>",
        description: "Switch the worker provider",
        category: "Models",
    },
    CommandSpec {
        name: "/model",
        usage: "/model <model>",
        description: "Switch the worker model",
        category: "Models",
    },
    CommandSpec {
        name: "/worker",
        usage: "/worker <model>",
        description: "Switch the worker model",
        category: "Models",
    },
    CommandSpec {
        name: "/planner-provider",
        usage: "/planner-provider <provider>",
        description: "Switch the planner provider",
        category: "Models",
    },
    CommandSpec {
        name: "/planner",
        usage: "/planner <model>",
        description: "Switch the planner model",
        category: "Models",
    },
    CommandSpec {
        name: "/width",
        usage: "/width [full|comfortable|<columns>]",
        description: "Prose measure: fill the pane, or cap it for readability",
        category: "Settings",
    },
    CommandSpec {
        name: "/activity",
        usage: "/activity [show|autohide|off]",
        description: "Run-history pane: pinned, revealed after a run, or hidden",
        category: "Settings",
    },
    CommandSpec {
        name: "/keys",
        usage: "/keys",
        description: "List every keyboard shortcut this terminal can send",
        category: "Settings",
    },
    CommandSpec {
        name: "/autocompact",
        usage: "/autocompact",
        description: "Toggle automatic context compaction",
        category: "Settings",
    },
    CommandSpec {
        name: "/allow",
        usage: "/allow <tool-pattern>",
        description: "Trust a tool pattern for this session",
        category: "Safety",
    },
    CommandSpec {
        name: "/deny",
        usage: "/deny <tool-pattern>",
        description: "Record a denied tool pattern",
        category: "Safety",
    },
    CommandSpec {
        name: "/agent",
        usage: "/agent",
        description: "Enable trusted agent mode after confirmation",
        category: "Safety",
    },
    CommandSpec {
        name: "/memory",
        usage: "/memory <recall|remember|forget> <text>",
        description: "Use the durable memory service",
        category: "Utilities",
    },
    CommandSpec {
        name: "/time",
        usage: "/time [relative expression]",
        description: "Show or resolve date and time",
        category: "Utilities",
    },
    CommandSpec {
        name: "/exit",
        usage: "/exit",
        description: "Close the workspace",
        category: "Workspace",
    },
];

#[derive(Default)]
struct PaletteState {
    query: String,
    selected: usize,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Mode {
    Build,
    Plan,
}

impl Mode {
    fn label(self) -> &'static str {
        match self {
            Self::Build => "BUILD",
            Self::Plan => "PLAN",
        }
    }
}

/// The number of transcript messages the Workspace retains.
///
/// The retained transcript is the headline feature, so the cap is generous. It
/// exists because the buffer was previously unbounded: a long session grew until
/// the process was killed, which is a worse outcome than visible elision.
/// Requirement 4.5.
const MAX_RETAINED_MESSAGES: usize = 500;

/// Prompt history entries retained for recall.
const MAX_PROMPT_HISTORY: usize = 200;

/// Role used for the single retained elision marker.
const ELIDED_ROLE: &str = "ELIDED";

/// How much room the run-history pane is allowed to take.
///
/// The activity pane held 30% of the width unconditionally in the wide layout,
/// even with nothing in it. That is a lot of the screen spent on chrome during
/// the part of a turn when the streamed response most needs the room.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
enum ActivityVisibility {
    /// Always pinned open.
    Show,
    /// Hidden while a turn is in flight, revealed once it finishes.
    ///
    /// The default: while work is running the response is what matters, and the
    /// record of what it did is what matters afterwards.
    #[default]
    AutoHide,
    /// Never shown.
    Off,
}

impl ActivityVisibility {
    fn label(self) -> &'static str {
        match self {
            Self::Show => "show",
            Self::AutoHide => "autohide",
            Self::Off => "off",
        }
    }

    fn parse(input: &str) -> Option<Self> {
        match input.trim().to_ascii_lowercase().as_str() {
            "show" | "on" | "always" => Some(Self::Show),
            "autohide" | "auto" => Some(Self::AutoHide),
            "off" | "hide" | "never" => Some(Self::Off),
            _ => None,
        }
    }

    /// Cycle for a bare `/activity` with no argument.
    fn next(self) -> Self {
        match self {
            Self::AutoHide => Self::Show,
            Self::Show => Self::Off,
            Self::Off => Self::AutoHide,
        }
    }
}

struct Message {
    role: String,
    text: String,
    /// Rendered lines, cached against the text length and width they were
    /// produced from.
    ///
    /// `draw_transcript` runs on every dirty frame — every streamed delta and
    /// every 250ms while busy — and previously re-parsed Markdown for the whole
    /// transcript each time, so redraw cost grew with session length. Width is
    /// part of the key because wrapping depends on it: a resized pane must
    /// re-render, and only then.
    rendered: std::cell::RefCell<Option<(usize, usize, Vec<Line<'static>>)>>,
}

impl Message {
    fn new(role: impl Into<String>, text: impl Into<String>) -> Self {
        Self {
            role: role.into(),
            text: text.into(),
            rendered: std::cell::RefCell::new(None),
        }
    }

    /// Append streamed text and drop the stale render.
    fn append(&mut self, delta: &str) {
        self.text.push_str(delta);
        self.rendered.replace(None);
    }

    /// Rendered lines for a given width, computed once per text revision.
    fn lines(&self, width: usize) -> Vec<Line<'static>> {
        let mut cache = self.rendered.borrow_mut();
        if let Some((len, cached_width, lines)) = cache.as_ref()
            && *len == self.text.len()
            && *cached_width == width
        {
            return lines.clone();
        }
        let lines = markdown_lines_wrapped(&self.text, width);
        *cache = Some((self.text.len(), width, lines.clone()));
        lines
    }
}

struct Activity {
    call_id: Option<String>,
    name: String,
    detail: String,
    state: ActivityState,
    started: Instant,
    elapsed: Option<Duration>,
}

enum ActivityState {
    Running,
    Passed,
    Failed,
}

struct PendingApproval {
    tool: String,
    detail: String,
    response: Option<tokio::sync::oneshot::Sender<ApprovalDecision>>,
    enables_agent_mode: bool,
}

struct App {
    input: String,
    cursor: usize,
    messages: Vec<Message>,
    activities: Vec<Activity>,
    current_assistant: Option<usize>,
    mode: Mode,
    busy: bool,
    /// Rows between the top of the rendered transcript and the top of the
    /// viewport. Zero is the very beginning of the conversation.
    ///
    /// Anchored to the content rather than to the bottom. It used to count back
    /// from the newest line, which meant the bottom was the reference point —
    /// and while a response streams the bottom moves. A reader who scrolled back
    /// to re-read something watched it drift off the top at exactly the rate
    /// output arrived, because "twenty lines from the end" names a different
    /// twenty lines every time the end moves. Measuring from the top instead
    /// makes an append a no-op for a detached view.
    scroll_offset: usize,
    context_percent: u16,
    active_agent: String,
    approval: Option<PendingApproval>,
    palette: Option<PaletteState>,
    follow_output: bool,
    history: Vec<String>,
    history_index: Option<usize>,
    history_draft: String,
    shell_mode: bool,
    /// Whether the terminal's mouse reporting is claimed by the app.
    ///
    /// On by default, which is not the obvious choice: claiming the mouse costs
    /// the terminal's own click-drag selection. It is the right one because
    /// leaving the wheel to the terminal is actively destructive rather than
    /// merely inert. Screenshots of the workspace before and after eight wheel
    /// notches in Apple Terminal show the whole drawn frame pushed six rows down
    /// the window with blank space above it: the terminal scrolls its own buffer,
    /// displacing a frame the app still believes it owns. Every later diffed
    /// redraw then lands at the wrong row and the transcript interleaves old and
    /// new text. With reporting claimed the frame stays put and the transcript
    /// scrolls, which is what the wheel is for.
    ///
    /// Selection is one modifier away — `Fn` in Apple Terminal, `Option` in
    /// iTerm2, `Shift` in most others — and `Ctrl+R` hands the mouse back
    /// wholesale.
    mouse_capture: bool,
    /// Lines the transcript moves per wheel notch.
    ///
    /// Configurable because terminals disagree about how many events a physical
    /// notch produces — some amplify, some send exactly one — and the app cannot
    /// tell which. Three matches `vim` and is what Bubble Tea and Claude Code
    /// both default to.
    wheel_lines: usize,
    /// Largest useful scroll offset, in lines, recorded by the last draw.
    ///
    /// The key handler needs this to clamp. Without it `scroll` grew without
    /// bound on PageUp — the render clamped for display but never wrote the
    /// bound back — so overshooting the top left dead range that PageDown had to
    /// walk through before the view moved at all. That reads as "scrolling is
    /// broken" when it is really an unclamped counter.
    max_scroll: std::cell::Cell<usize>,
    /// Visible transcript height in lines, recorded by the last draw, so a page
    /// step is an actual page rather than a fixed guess.
    viewport: std::cell::Cell<usize>,
    /// Row offset of every message label in the rendered transcript, recorded by
    /// the last draw.
    ///
    /// Semantic navigation moves between these instead of a fixed number of
    /// lines, so one keystroke lands on the start of a response whatever its
    /// length. Only the renderer knows where the boundaries fell, because only
    /// it knows the wrapped height of each message at the current width.
    message_rows: std::cell::RefCell<Vec<usize>>,
    activity_visibility: ActivityVisibility,
    /// Optional cap on prose measure, in columns.
    ///
    /// `None` fills the pane. A cap is better for reading — long lines make the
    /// eye lose its place on the return sweep — but on a wide terminal the
    /// leftover space looks like a panel that failed to close, so filling is the
    /// less surprising default and `/width` opts into the cap.
    prose_width: Option<u16>,
    /// Every chord the workspace answers, resolved for this terminal.
    ///
    /// Held on the app so the key handler, the footer, and `/keys` all read the
    /// same table; they used to spell the shortcuts out separately and had
    /// already drifted.
    keys: crate::tui_keys::ActionRegistry,
    /// Set when the next frame must repaint every cell rather than a diff.
    ///
    /// Needed because the terminal can move the drawn frame out from under the
    /// app — by scrolling its own buffer — and nothing reports that, so recovery
    /// has to be something the developer can ask for.
    force_redraw: bool,
    /// When a redraw was last asked for, so a second request in quick succession
    /// can mean "and clear the conversation too".
    last_redraw_request: Option<Instant>,
    task_abort: Option<tokio::task::AbortHandle>,
}

impl App {
    fn new() -> Self {
        Self {
            input: String::new(),
            cursor: 0,
            messages: Vec::new(),
            activities: Vec::new(),
            current_assistant: None,
            mode: Mode::Build,
            busy: false,
            scroll_offset: 0,
            context_percent: 0,
            active_agent: "idle".into(),
            approval: None,
            palette: None,
            follow_output: true,
            history: Vec::new(),
            history_index: None,
            history_draft: String::new(),
            shell_mode: false,
            mouse_capture: true,
            wheel_lines: 3,
            max_scroll: std::cell::Cell::new(0),
            viewport: std::cell::Cell::new(0),
            message_rows: std::cell::RefCell::new(Vec::new()),
            activity_visibility: ActivityVisibility::default(),
            prose_width: None,
            keys: crate::tui_keys::ActionRegistry::detect(),
            force_redraw: false,
            last_redraw_request: None,
            task_abort: None,
        }
    }

    /// Whether the run-history pane should occupy space this frame.
    ///
    /// `AutoHide` deliberately hides *while* busy rather than after: the
    /// streamed response is what the developer is reading during a turn, and the
    /// record of what ran is what they want once it is done.
    fn show_activity(&self) -> bool {
        match self.activity_visibility {
            ActivityVisibility::Off => false,
            ActivityVisibility::Show => true,
            ActivityVisibility::AutoHide => !self.busy && !self.activities.is_empty(),
        }
    }

    /// The measure to wrap prose at, given the available content width.
    fn prose_measure(&self, content_width: usize) -> usize {
        match self.prose_width {
            Some(cap) => content_width.min(cap as usize).max(20),
            None => content_width,
        }
    }

    /// One page, in lines: the viewport minus an overlap row so the reader keeps
    /// a line of context across the jump.
    fn page_step(&self) -> usize {
        self.viewport.get().saturating_sub(1).max(1)
    }

    /// Half a page, in lines — the step `PageUp`/`PageDown` take.
    ///
    /// A whole page leaves no overlap, so every jump forces the reader to
    /// re-find their place. Half keeps the previous half on screen, which is why
    /// both Claude Code's fullscreen renderer and grok's `Ctrl+U`/`Ctrl+D` move
    /// by half a screen. The full page is still reachable on `Shift+PageUp`.
    fn half_page_step(&self) -> usize {
        (self.viewport.get() / 2).max(1)
    }

    /// Scroll towards the start of the conversation, stopping at the first line.
    fn scroll_up(&mut self, lines: usize) {
        self.scroll_offset = self.top_offset().saturating_sub(lines);
        self.follow_output = false;
    }

    /// Scroll towards the newest output.
    ///
    /// Reaching the end re-arms following, so scrolling back down by hand has
    /// the same result as jumping there.
    fn scroll_down(&mut self, lines: usize) {
        let max = self.max_scroll.get();
        self.scroll_offset = self.top_offset().saturating_add(lines).min(max);
        self.follow_output = self.scroll_offset >= max;
    }

    /// Jump to the very beginning of the conversation.
    fn scroll_to_start(&mut self) {
        self.scroll_offset = 0;
        self.follow_output = false;
    }

    /// Jump to the newest output and resume following it.
    fn scroll_to_end(&mut self) {
        self.scroll_offset = self.max_scroll.get();
        self.follow_output = true;
    }

    /// Rows between the top of the transcript and the top of the viewport.
    ///
    /// While following, the effective position is the tail whatever
    /// `scroll_offset` last held, because the renderer pins it there. Resolving
    /// that here means a scroll away from the bottom starts from where the reader
    /// can actually see, not from a stale offset.
    fn top_offset(&self) -> usize {
        let max = self.max_scroll.get();
        if self.follow_output {
            max
        } else {
            self.scroll_offset.min(max)
        }
    }

    /// Swap the wheel for native text selection, or back.
    ///
    /// Both messages name the cost, because in the alternate screen this is a
    /// real trade rather than a setting with a better side: the terminal either
    /// forwards wheel events to the app or keeps them, and it cannot do both.
    /// The modifier that forces native selection anyway is terminal-specific, so
    /// the message names the one that works here.
    fn toggle_mouse_capture(&mut self) {
        self.mouse_capture = !self.mouse_capture;
        // Claiming the mouse usually follows the terminal having scrolled the
        // frame out of position, so repaint rather than diff onto a stale buffer.
        self.force_redraw = true;
        let chord = self
            .keys
            .advertised_key(crate::tui_keys::ActionId::ToggleMouseCapture)
            .map(|key| key.display())
            .unwrap_or_else(|| "/mouse".into());
        if self.mouse_capture {
            let modifier = self.keys.terminal().native_selection_modifier();
            self.push_system(format!(
                "Mouse wheel **on** — it scrolls the transcript. The terminal no \
                 longer gets click-drag selection, so hold `{modifier}` while \
                 dragging to select natively, or press `{chord}` to hand the \
                 mouse back."
            ));
        } else {
            self.push_system(format!(
                "Mouse wheel **off** — select and copy with the mouse as usual. \
                 The terminal now owns the wheel, and in the alternate screen it \
                 will scroll its own buffer and push this frame out of place; \
                 press `Ctrl+L` to repaint, or `{chord}` to take the wheel back. \
                 Scroll with `PageUp`/`PageDown` meanwhile."
            ));
        }
    }

    /// Repaint every cell on the next frame, and report it.
    ///
    /// `Ctrl+L` means "redraw" almost everywhere, and a developer whose frame the
    /// terminal has displaced will reach for it. It used to clear the
    /// conversation outright, so the conventional reflex for a corrupted screen
    /// destroyed the transcript instead of repairing it. Clearing now takes a
    /// second press, which is also how Claude Code resolves the same collision.
    fn request_redraw(&mut self) -> bool {
        self.force_redraw = true;
        let recent = self
            .last_redraw_request
            .is_some_and(|at| at.elapsed() < Duration::from_secs(2));
        if recent {
            self.last_redraw_request = None;
            return true;
        }
        self.last_redraw_request = Some(Instant::now());
        false
    }

    /// Put a message boundary at the top of the viewport.    ///
    /// Moving by response rather than by line means one keystroke reaches the
    /// start of the previous answer whether it is three lines or three hundred.
    /// Running off either end lands at that end instead of doing nothing, which
    /// is what a reader holding the key expects.
    fn scroll_to_message(&mut self, backwards: bool) {
        let current = self.top_offset();
        let target = {
            let rows = self.message_rows.borrow();
            if backwards {
                rows.iter().rev().copied().find(|&row| row < current)
            } else {
                rows.iter().copied().find(|&row| row > current)
            }
        };
        match target {
            Some(row) => {
                self.scroll_offset = row.min(self.max_scroll.get());
                self.follow_output = self.scroll_offset >= self.max_scroll.get();
            }
            None if backwards => self.scroll_to_start(),
            None => self.scroll_to_end(),
        }
    }

    fn push_system(&mut self, text: impl Into<String>) {
        self.push_message(Message::new("ZAVORA", text.into()));
    }

    /// Append a message, enforcing the retention cap.
    ///
    /// When the cap is reached, the oldest messages are dropped and a single
    /// marker records how many. Silent truncation would be worse than the
    /// unbounded growth it replaces: the developer would have no way to know
    /// the transcript was no longer complete. Requirement 4.5.
    fn push_message(&mut self, message: Message) -> usize {
        self.messages.push(message);

        if self.messages.len() > MAX_RETAINED_MESSAGES {
            // One slot is reserved for the marker itself, so keep the newest
            // `MAX - 1` real messages.
            let keep = MAX_RETAINED_MESSAGES - 1;
            let remove = self.messages.len() - keep;

            // Carry forward any previous count; the old marker is not itself an
            // elided message, so it must not be counted as one.
            let previous = self
                .messages
                .first()
                .filter(|first| first.role == ELIDED_ROLE)
                .and_then(|first| first.text.split_whitespace().next()?.parse::<usize>().ok());
            let removed_real = remove - usize::from(previous.is_some());
            let elided = previous.unwrap_or(0) + removed_real;

            self.messages.drain(0..remove);
            self.messages.insert(
                0,
                Message::new(ELIDED_ROLE, format!("{elided} earlier messages elided")),
            );

            // Keep the streaming target pointing at the same message. `remove`
            // entries went away and one marker arrived.
            self.current_assistant = self
                .current_assistant
                .and_then(|index| index.checked_sub(remove))
                .map(|index| index + 1);
        }

        self.messages.len() - 1
    }

    fn replace_input(&mut self, input: impl Into<String>) {
        self.input = input.into();
        self.cursor = self.input.len();
    }

    fn remember_input(&mut self, input: &str) {
        if self.history.last().is_none_or(|last| last != input) {
            self.history.push(input.to_string());
            if self.history.len() > MAX_PROMPT_HISTORY {
                let overflow = self.history.len() - MAX_PROMPT_HISTORY;
                self.history.drain(0..overflow);
            }
        }
        self.history_index = None;
        self.history_draft.clear();
    }

    fn history_previous(&mut self) {
        if self.history.is_empty() {
            return;
        }
        let index = match self.history_index {
            Some(index) => index.saturating_sub(1),
            None => {
                self.history_draft = self.input.clone();
                self.history.len() - 1
            }
        };
        self.history_index = Some(index);
        self.replace_input(self.history[index].clone());
    }

    fn history_next(&mut self) {
        let Some(index) = self.history_index else {
            return;
        };
        if index + 1 < self.history.len() {
            let next = index + 1;
            self.history_index = Some(next);
            self.replace_input(self.history[next].clone());
        } else {
            self.history_index = None;
            self.replace_input(self.history_draft.clone());
            self.history_draft.clear();
        }
    }

    fn apply(&mut self, event: UiEvent) {
        match event {
            UiEvent::AgentChanged(agent) => self.active_agent = agent,
            UiEvent::System(text) => self.push_system(text),
            UiEvent::TextDelta { author, text } => {
                let index = match self.current_assistant {
                    Some(index) if self.messages[index].role == author => index,
                    _ => {
                        let index = self.push_message(Message::new(author, String::new()));
                        self.current_assistant = Some(index);
                        index
                    }
                };
                self.messages[index].append(&text);
            }
            UiEvent::ToolStarted {
                call_id,
                name,
                detail,
            } => self.activities.push(Activity {
                call_id,
                name,
                detail,
                state: ActivityState::Running,
                started: Instant::now(),
                elapsed: None,
            }),
            UiEvent::ToolFinished {
                call_id,
                name,
                success,
                detail,
            } => {
                if let Some(item) =
                    self.activities
                        .iter_mut()
                        .rev()
                        .find(|item| match (&call_id, &item.call_id) {
                            (Some(result_id), Some(activity_id)) => result_id == activity_id,
                            _ => item.name == name && matches!(item.state, ActivityState::Running),
                        })
                {
                    item.state = if success {
                        ActivityState::Passed
                    } else {
                        ActivityState::Failed
                    };
                    item.detail = detail;
                    item.elapsed = Some(item.started.elapsed());
                }
            }
            UiEvent::Error(error) => self.push_system(format!("Runtime error: {error}")),
            UiEvent::Completed(_) => {
                self.busy = false;
                self.current_assistant = None;
                self.active_agent = "idle".into();
                self.task_abort = None;
            }
        }
    }
}

fn matching_commands(query: &str) -> Vec<&'static CommandSpec> {
    let query = query.trim().trim_start_matches('/').to_ascii_lowercase();
    let terms = query.split_whitespace().collect::<Vec<_>>();
    COMMAND_SPECS
        .iter()
        .filter(|spec| {
            if terms.is_empty() {
                return true;
            }
            let haystack = format!(
                "{} {} {} {}",
                spec.name, spec.usage, spec.description, spec.category
            )
            .to_ascii_lowercase();
            terms.iter().all(|term| haystack.contains(term))
        })
        .collect()
}

fn slash_suggestions(input: &str) -> Vec<&'static CommandSpec> {
    if !input.starts_with('/') || input.contains('\n') || input.contains(char::is_whitespace) {
        return Vec::new();
    }
    let query = input.trim_start_matches('/').to_ascii_lowercase();
    COMMAND_SPECS
        .iter()
        .filter(|spec| spec.name.trim_start_matches('/').starts_with(&query))
        .take(7)
        .collect()
}

fn command_input(spec: &CommandSpec) -> String {
    if spec.usage.contains('<') || spec.usage.contains('[') {
        format!("{} ", spec.name)
    } else {
        spec.name.to_string()
    }
}

pub async fn run_tui_chat(
    mut cfg: RuntimeConfig,
    retrieval: Arc<dyn RetrievalService>,
    runtime_tools: ResolvedRuntimeTools,
    confirmation: ToolConfirmationSettings,
    telemetry: &TelemetrySink,
) -> Result<()> {
    let mut runtime_tools = Arc::new(runtime_tools);
    let session_service = build_session_service(&cfg).await?;
    let (runner, mut provider, model) = build_single_runner_for_chat(
        &cfg,
        session_service.clone(),
        runtime_tools.as_ref(),
        &confirmation,
        telemetry,
    )
    .await?;
    cfg.provider = provider;
    cfg.model = Some(model.clone());
    let mut runner = Arc::new(runner);
    let mut model_name = model;
    let telemetry = telemetry.clone();
    let (ui_tx, mut ui_rx) = tokio::sync::mpsc::unbounded_channel();
    let mut approval_rx = install_approval_bridge();

    enable_raw_mode().context("failed to enable terminal raw mode")?;
    let mut stdout = io::stdout();
    // Mouse reporting is claimed here to match `App::new()`. Leaving the wheel to
    // the terminal is not a neutral choice: in the alternate screen the terminal
    // scrolls its own buffer and displaces the frame the app is drawing into, and
    // every diffed redraw after that lands at the wrong row. `Ctrl+R` hands the
    // mouse back for anyone who wants native selection more than the wheel.
    if let Err(error) = execute!(
        stdout,
        EnterAlternateScreen,
        EnableBracketedPaste,
        EnableMouseCapture
    ) {
        disable_raw_mode().ok();
        return Err(error).context("failed to enter terminal workspace");
    }
    // From here the terminal is in states the shell cannot undo by itself. Arm the
    // restore before anything else can fail: a panic or a signal from this point
    // on would otherwise leave raw mode and mouse reporting on, and a terminal
    // still reporting motion prints escape sequences as text into the shell.
    crate::tui_restore::arm();
    let backend = CrosstermBackend::new(stdout);
    let mut terminal = match ratatui::Terminal::new(backend) {
        Ok(terminal) => terminal,
        Err(error) => {
            crate::tui_restore::restore();
            return Err(error).context("failed to initialize terminal workspace");
        }
    };
    let mut app = App::new();
    // Mirrors the terminal's actual mouse-reporting state so the loop only
    // issues a control sequence when it genuinely changes. Matches both
    // `App::new()` and the sequence issued above.
    let mut mouse_capture_active = true;
    let workspace = std::env::current_dir().unwrap_or_default();
    let mut checkpoint_store = CheckpointStore::load_from_disk(&workspace);
    let mut last_context_refresh = Instant::now() - Duration::from_secs(1);
    let mut last_animation = Instant::now();
    let mut dirty = true;

    let local_tasks = tokio::task::LocalSet::new();
    let result = local_tasks
        .run_until(async {
            loop {
                while let Ok(event) = ui_rx.try_recv() {
                    app.apply(event);
                    dirty = true;
                }
                if app.approval.is_none()
                    && let Ok(request) = approval_rx.try_recv()
                {
                    app.approval = Some(PendingApproval {
                        tool: request.tool,
                        detail: request.detail,
                        response: Some(request.response),
                        enables_agent_mode: false,
                    });
                    dirty = true;
                }
                if last_context_refresh.elapsed() >= Duration::from_millis(750) {
                    if let Ok(events) = snapshot_session_events(&session_service, &cfg).await {
                        let usage = compute_context_usage(
                            &events,
                            &provider.to_string(),
                            &cfg.worker_model,
                        );
                        let context_percent = (usage.utilization() * 100.0).min(100.0) as u16;
                        if context_percent != app.context_percent {
                            app.context_percent = context_percent;
                            dirty = true;
                        }

                        // Automatic compaction. The Workspace previously showed
                        // context usage climbing but never acted on it, so a long
                        // session hit the provider's limit instead of compacting.
                        // Only between turns: compacting mid-turn would rewrite
                        // the session the in-flight request is reading.
                        // Requirement 12.3.
                        if !app.busy
                            && cfg.auto_compact_enabled
                            && usage.utilization() >= cfg.compaction_threshold
                        {
                            app.active_agent = "compacting".into();
                            match crate::compact::auto_compact(&session_service, &cfg).await {
                                Ok(message) => app.push_system(format!("Compacted: {message}")),
                                Err(error) => {
                                    app.push_system(format!("Auto-compaction failed: {error}"))
                                }
                            }
                            app.active_agent = "idle".into();
                            dirty = true;
                        }
                    }
                    last_context_refresh = Instant::now();
                }

                // A capability enabled during the last turn added MCP servers to
                // the profile, and the sealed tool surface predates them. Reseal
                // between turns so the capability works in this session instead of
                // asking the developer to restart. Never mid-turn: the in-flight
                // request is holding the current surface.
                if !app.busy && crate::capabilities::take_surface_stale() {
                    app.active_agent = "connecting".into();
                    let _ = terminal.draw(|frame| draw(frame, &app, &cfg));

                    match crate::config::reload_mcp_servers(&mut cfg) {
                        Ok(declared) => {
                            let before = runtime_tools.tools().len();
                            let resolved =
                                Arc::new(crate::runner::resolve_runtime_tools(&cfg).await);
                            let failures = resolved.connect_failures().len();
                            let gained = resolved.tools().len().saturating_sub(before);

                            match build_single_runner_for_chat(
                                &cfg,
                                session_service.clone(),
                                resolved.as_ref(),
                                &confirmation,
                                &telemetry,
                            )
                            .await
                            {
                                Ok((next_runner, _, _)) => {
                                    runner = Arc::new(next_runner);
                                    runtime_tools = resolved;
                                    // Configured and connected are reported apart:
                                    // a declared server can still refuse to answer.
                                    let mut message = format!(
                                        "Capability activated — {declared} MCP server{} configured, \
                                         {gained} new tool{} available now.",
                                        if declared == 1 { "" } else { "s" },
                                        if gained == 1 { "" } else { "s" },
                                    );
                                    if failures > 0 {
                                        message.push_str(&format!(
                                            "\n\n{failures} server{} did not answer; run `/mcp` to see which.",
                                            if failures == 1 { "" } else { "s" }
                                        ));
                                    }
                                    app.push_system(message);
                                }
                                Err(error) => app.push_system(format!(
                                    "Servers were configured, but rebuilding the agent failed, so \
                                     the previous tools remain active.\n\n`{}`",
                                    crate::error::format_cli_error(
                                        &error,
                                        cfg.show_sensitive_config
                                    )
                                )),
                            }
                        }
                        Err(error) => app.push_system(format!(
                            "Servers were configured, but the profile could not be re-read, so \
                             they are not active yet.\n\n`{error}`"
                        )),
                    }
                    app.active_agent = "idle".into();
                    dirty = true;
                }
                if app.busy && last_animation.elapsed() >= Duration::from_millis(250) {
                    dirty = true;
                    last_animation = Instant::now();
                }
                // Reconcile the terminal's mouse reporting with the requested
                // state. Done here rather than in the command handler so there is
                // one place that owns the terminal mode.
                if app.mouse_capture != mouse_capture_active {
                    let result = if app.mouse_capture {
                        execute!(terminal.backend_mut(), EnableMouseCapture)
                    } else {
                        execute!(terminal.backend_mut(), DisableMouseCapture)
                    };
                    match result {
                        Ok(()) => mouse_capture_active = app.mouse_capture,
                        Err(error) => {
                            app.push_system(format!("Could not change mouse capture: {error}"));
                            app.mouse_capture = mouse_capture_active;
                        }
                    }
                    dirty = true;
                }
                if dirty {
                    // A full repaint discards ratatui's back buffer, which is
                    // the only way back from a frame the terminal has displaced.
                    // Scrolling the terminal's own buffer moves the drawn frame
                    // without the app knowing, and from then on a diffed redraw
                    // paints every changed cell at the wrong row, interleaving
                    // old and new text.
                    if app.force_redraw {
                        terminal.clear()?;
                        app.force_redraw = false;
                    }
                    terminal.draw(|frame| draw(frame, &app, &cfg))?;
                    dirty = false;
                }
                if event::poll(Duration::from_millis(40))? {
                    match event::read()? {
                        TerminalEvent::Key(key) if key.kind == KeyEventKind::Press => {
                            let should_exit = handle_key(
                                key,
                                &mut app,
                                &mut runner,
                                &mut cfg,
                                &mut provider,
                                &mut model_name,
                                &session_service,
                                &mut checkpoint_store,
                                retrieval.clone(),
                                telemetry.clone(),
                                ui_tx.clone(),
                                runtime_tools.clone(),
                                &confirmation,
                            )
                            .await;
                            dirty = true;
                            if should_exit {
                                break;
                            }
                        }
                        TerminalEvent::Paste(text)
                            if !app.busy && app.approval.is_none() && app.palette.is_none() =>
                        {
                            app.input.insert_str(app.cursor, &text);
                            app.cursor += text.len();
                            dirty = true;
                        }
                        TerminalEvent::Resize(_, _) => dirty = true,
                        TerminalEvent::Mouse(mouse) => {
                            match mouse.kind {
                                MouseEventKind::ScrollUp => app.scroll_up(app.wheel_lines),
                                MouseEventKind::ScrollDown => app.scroll_down(app.wheel_lines),
                                MouseEventKind::Down(MouseButton::Left) => {
                                    let size = terminal.size()?;
                                    handle_mouse_click(
                                        &mut app,
                                        mouse.column,
                                        mouse.row,
                                        Rect::new(0, 0, size.width, size.height),
                                    );
                                }
                                _ => {}
                            }
                            dirty = true;
                        }
                        _ => {}
                    }
                }
            }
            Ok::<(), anyhow::Error>(())
        })
        .await;

    clear_approval_bridge();
    // Hand the terminal back through the same idempotent path the panic hook and
    // signal handler use, so a signal arriving mid-teardown cannot double up and
    // a panic in here still leaves a usable shell.
    crate::tui_restore::restore();
    terminal.show_cursor().ok();
    result
}

fn handle_mouse_click(app: &mut App, column: u16, row: u16, area: Rect) {
    if let Some(palette) = app.palette.as_mut() {
        let popup = centered(72, 72, area);
        if column < popup.x || column >= popup.right() || row < popup.y || row >= popup.bottom() {
            app.palette = None;
            return;
        }
        let first_item_row = popup.y.saturating_add(3);
        if row < first_item_row {
            return;
        }
        let matches = matching_commands(&palette.query);
        let visible = popup.height.saturating_sub(6) as usize;
        let selected = palette.selected.min(matches.len().saturating_sub(1));
        let start = selected.saturating_sub(visible.saturating_sub(1));
        let clicked = start + row.saturating_sub(first_item_row) as usize;
        if clicked < matches.len() {
            palette.selected = clicked;
        }
    } else if row < 2 && app.approval.is_none() && !app.busy {
        app.mode = if app.mode == Mode::Build {
            Mode::Plan
        } else {
            Mode::Build
        };
    }
}

#[allow(clippy::too_many_arguments)]
/// Apply a navigation action, reporting whether it was one.
///
/// Navigation is the part of the key map dispatched from the registry rather
/// than from a hand-written arm, so the chords, the footer, and `/keys` cannot
/// disagree. Everything else — sending, cancelling, composer editing — carries
/// enough surrounding logic that a table entry would only describe it, and those
/// stay as explicit arms below.
fn apply_navigation(app: &mut App, action: crate::tui_keys::ActionId) -> bool {
    use crate::tui_keys::ActionId;
    match action {
        ActionId::ScrollLineUp => app.scroll_up(1),
        ActionId::ScrollLineDown => app.scroll_down(1),
        ActionId::ScrollHalfPageUp => {
            let step = app.half_page_step();
            app.scroll_up(step);
        }
        ActionId::ScrollHalfPageDown => {
            let step = app.half_page_step();
            app.scroll_down(step);
        }
        ActionId::ScrollPageUp => {
            let step = app.page_step();
            app.scroll_up(step);
        }
        ActionId::ScrollPageDown => {
            let step = app.page_step();
            app.scroll_down(step);
        }
        ActionId::ScrollToStart => app.scroll_to_start(),
        ActionId::ScrollToEnd => app.scroll_to_end(),
        ActionId::PrevMessage => app.scroll_to_message(true),
        ActionId::NextMessage => app.scroll_to_message(false),
        // Not navigation, but it belongs with it: the wheel is a scroll control,
        // and reaching it by chord matters most when the wheel is dead.
        ActionId::ToggleMouseCapture => app.toggle_mouse_capture(),
        // Not navigation: leave it to the arms below.
        _ => return false,
    }
    true
}

#[allow(clippy::too_many_arguments)]
async fn handle_key(
    key: KeyEvent,
    app: &mut App,
    runner: &mut Arc<Runner>,
    cfg: &mut RuntimeConfig,
    provider: &mut crate::cli::Provider,
    model_name: &mut String,
    session_service: &Arc<dyn adk_session::SessionService>,
    checkpoint_store: &mut CheckpointStore,
    retrieval: Arc<dyn RetrievalService>,
    telemetry: TelemetrySink,
    tx: tokio::sync::mpsc::UnboundedSender<UiEvent>,
    runtime_tools: Arc<ResolvedRuntimeTools>,
    confirmation: &ToolConfirmationSettings,
) -> bool {
    if let Some(approval) = app.approval.as_mut() {
        let decision = match key.code {
            KeyCode::Char('y') => Some(ApprovalDecision::AllowOnce),
            KeyCode::Char('t') => Some(ApprovalDecision::TrustSession),
            KeyCode::Char('n') | KeyCode::Esc => Some(ApprovalDecision::Deny),
            _ => None,
        };
        if let Some(decision) = decision {
            if let Some(response) = approval.response.take() {
                let _ = response.send(decision);
            }
            if approval.enables_agent_mode
                && matches!(
                    decision,
                    ApprovalDecision::AllowOnce | ApprovalDecision::TrustSession
                )
            {
                for tool in ["fs_read", "fs_write", "execute_bash"] {
                    crate::tools::confirming::trust_tool(tool);
                }
                app.push_system("Agent mode enabled for this session.");
            }
            app.approval = None;
        }
        return false;
    }
    if app.palette.is_some() {
        let mut accepted = None;
        if let Some(palette) = app.palette.as_mut() {
            let matches = matching_commands(&palette.query);
            match (key.code, key.modifiers) {
                (KeyCode::Esc, _) => app.palette = None,
                (KeyCode::Up, _) => {
                    palette.selected = palette.selected.saturating_sub(1);
                }
                (KeyCode::Down, _) => {
                    palette.selected = (palette.selected + 1).min(matches.len().saturating_sub(1));
                }
                (KeyCode::Backspace, _) => {
                    palette.query.pop();
                    palette.selected = 0;
                }
                (KeyCode::Char('u'), KeyModifiers::CONTROL) => {
                    palette.query.clear();
                    palette.selected = 0;
                }
                (KeyCode::Enter, _) => {
                    accepted = matches
                        .get(palette.selected.min(matches.len().saturating_sub(1)))
                        .map(|spec| command_input(spec));
                }
                (KeyCode::Char(character), modifiers)
                    if !modifiers.contains(KeyModifiers::CONTROL) =>
                {
                    palette.query.push(character);
                    palette.selected = 0;
                }
                _ => {}
            }
        }
        if let Some(input) = accepted {
            app.palette = None;
            let submit_now = !input.ends_with(' ');
            app.replace_input(input);
            if submit_now {
                return submit_input(
                    app,
                    runner,
                    cfg,
                    provider,
                    model_name,
                    session_service,
                    checkpoint_store,
                    retrieval,
                    telemetry,
                    tx,
                    runtime_tools,
                    confirmation,
                )
                .await;
            }
        }
        return false;
    }
    // Navigation comes from the action registry, so the bound chords, the footer
    // hints, and the `/keys` listing are all the same table. A chord the registry
    // does not claim falls through to the arms below.
    if let Some(action) = app.keys.lookup(key.code, key.modifiers, app.busy)
        && apply_navigation(app, action)
    {
        return false;
    }
    match (key.code, key.modifiers) {
        (KeyCode::Char('c'), KeyModifiers::CONTROL) if !app.busy => return true,
        (KeyCode::Char('d'), KeyModifiers::CONTROL) if !app.busy && app.input.is_empty() => {
            return true;
        }
        (KeyCode::Char('p'), KeyModifiers::CONTROL) => app.palette = Some(PaletteState::default()),
        (KeyCode::Char('l'), KeyModifiers::CONTROL) if !app.busy => {
            // First press repaints; a second within two seconds clears.
            if app.request_redraw() {
                app.messages.clear();
                app.activities.clear();
                app.scroll_to_end();
            } else {
                app.push_system(
                    "Screen repainted. Press `Ctrl+L` again within two seconds to \
                     clear the conversation, or run `/clear`.",
                );
            }
        }
        (KeyCode::BackTab, _) => {
            app.mode = if app.mode == Mode::Build {
                Mode::Plan
            } else {
                Mode::Build
            };
        }
        (KeyCode::Esc, _) if app.busy => {
            if let Some(abort) = app.task_abort.take() {
                abort.abort();
            }
            runner.interrupt(&cfg.session_id);
            app.busy = false;
            app.current_assistant = None;
            app.active_agent = "idle".into();
            app.push_system("Run cancelled.");
        }
        (KeyCode::Esc, _) if app.shell_mode => {
            app.shell_mode = false;
            app.push_system("Direct shell mode disabled.");
        }
        (KeyCode::Up, _) if !app.busy => app.history_previous(),
        (KeyCode::Down, _) if !app.busy => app.history_next(),
        (KeyCode::Tab, _) if !app.busy => {
            if let Some(spec) = slash_suggestions(&app.input).first() {
                app.replace_input(command_input(spec));
            }
        }
        (KeyCode::Left, _) => app.cursor = previous_boundary(&app.input, app.cursor),
        (KeyCode::Right, _) => app.cursor = next_boundary(&app.input, app.cursor),
        (KeyCode::Char('b'), KeyModifiers::ALT) => {
            app.cursor = previous_word_boundary(&app.input, app.cursor)
        }
        (KeyCode::Char('f'), KeyModifiers::ALT) => {
            app.cursor = next_word_boundary(&app.input, app.cursor)
        }
        (KeyCode::Home, _) | (KeyCode::Char('a'), KeyModifiers::CONTROL) => app.cursor = 0,
        (KeyCode::End, _) | (KeyCode::Char('e'), KeyModifiers::CONTROL) => {
            app.cursor = app.input.len();
        }
        (KeyCode::Char('u'), KeyModifiers::CONTROL) => {
            app.input.clear();
            app.cursor = 0;
        }
        (KeyCode::Char('k'), KeyModifiers::CONTROL) => {
            app.input.truncate(app.cursor);
        }
        (KeyCode::Char('w'), KeyModifiers::CONTROL) if app.cursor > 0 => {
            let previous = previous_word_boundary(&app.input, app.cursor);
            app.input.drain(previous..app.cursor);
            app.cursor = previous;
        }
        (KeyCode::Backspace, _) if app.cursor > 0 => {
            let previous = previous_boundary(&app.input, app.cursor);
            app.input.drain(previous..app.cursor);
            app.cursor = previous;
        }
        (KeyCode::Delete, _) if app.cursor < app.input.len() => {
            let next = next_boundary(&app.input, app.cursor);
            app.input.drain(app.cursor..next);
        }
        (KeyCode::Char('j'), KeyModifiers::CONTROL) => {
            app.input.insert(app.cursor, '\n');
            app.cursor += 1;
        }
        (KeyCode::Enter, modifiers) if modifiers.contains(KeyModifiers::SHIFT) => {
            app.input.insert(app.cursor, '\n');
            app.cursor += 1;
        }
        (KeyCode::Enter, _) if !app.busy => {
            return submit_input(
                app,
                runner,
                cfg,
                provider,
                model_name,
                session_service,
                checkpoint_store,
                retrieval,
                telemetry,
                tx,
                runtime_tools,
                confirmation,
            )
            .await;
        }
        (KeyCode::Char(character), modifiers) if !modifiers.contains(KeyModifiers::CONTROL) => {
            app.input.insert(app.cursor, character);
            app.cursor += character.len_utf8();
        }
        _ => {}
    }
    false
}

#[allow(clippy::too_many_arguments)]
async fn submit_input(
    app: &mut App,
    runner: &mut Arc<Runner>,
    cfg: &mut RuntimeConfig,
    provider: &mut crate::cli::Provider,
    model_name: &mut String,
    session_service: &Arc<dyn adk_session::SessionService>,
    checkpoint_store: &mut CheckpointStore,
    retrieval: Arc<dyn RetrievalService>,
    telemetry: TelemetrySink,
    tx: tokio::sync::mpsc::UnboundedSender<UiEvent>,
    runtime_tools: Arc<ResolvedRuntimeTools>,
    confirmation: &ToolConfirmationSettings,
) -> bool {
    let mut raw = app.input.trim().to_string();
    if raw.is_empty() {
        return false;
    }
    app.remember_input(&raw);
    app.input.clear();
    app.cursor = 0;

    if raw == "/clear" {
        app.messages.clear();
        app.activities.clear();
        app.scroll_to_end();
        return false;
    }
    if raw == "/shell" || raw == "!" {
        app.shell_mode = !app.shell_mode;
        app.push_system(format!(
            "Direct shell mode {}. Commands run in the workspace using `{}`.",
            if app.shell_mode {
                "enabled"
            } else {
                "disabled"
            },
            std::env::var("SHELL").unwrap_or_else(|_| "sh".into())
        ));
        return false;
    }
    if app.shell_mode || raw.starts_with('!') {
        let command = raw.strip_prefix('!').unwrap_or(&raw).trim().to_string();
        if command.is_empty() {
            return false;
        }
        run_shell_command(app, cfg, command, session_service.clone(), telemetry, tx);
        return false;
    }
    if raw == "/mode" || raw.starts_with("/mode ") {
        let mode = raw.strip_prefix("/mode").unwrap_or_default();
        match mode.trim().to_ascii_lowercase().as_str() {
            "build" => app.mode = Mode::Build,
            "plan" => app.mode = Mode::Plan,
            "" => {
                app.mode = if app.mode == Mode::Build {
                    Mode::Plan
                } else {
                    Mode::Build
                };
            }
            _ => app.push_system("Usage: `/mode build` or `/mode plan`."),
        }
        return false;
    }
    if raw == "/copy" || raw.starts_with("/copy ") {
        let argument = raw.strip_prefix("/copy").unwrap_or_default().trim();
        let whole_transcript = matches!(argument, "all" | "transcript");
        match clipboard_payload(app, whole_transcript) {
            Some(payload) => match copy_to_clipboard(&payload) {
                Ok(binary) => app.push_system(format!(
                    "Copied {} ({} chars) to the clipboard via `{binary}`.",
                    if whole_transcript {
                        "the transcript"
                    } else {
                        "the last response"
                    },
                    payload.chars().count()
                )),
                Err(error) => app.push_system(format!("Copy failed: {error}")),
            },
            None => app.push_system(if whole_transcript {
                "Nothing to copy yet."
            } else {
                "No response to copy yet. `/copy all` copies the whole transcript."
            }),
        }
        return false;
    }
    if raw == "/width" || raw.starts_with("/width ") {
        let argument = raw.strip_prefix("/width").unwrap_or_default().trim();
        match argument {
            "" => {
                let current = match app.prose_width {
                    Some(cap) => format!("capped at {cap} columns"),
                    None => "filling the pane".to_string(),
                };
                app.push_system(format!(
                    "Prose measure is **{current}**. Use `/width full`, \
                     `/width comfortable`, or `/width <columns>`."
                ));
            }
            "full" | "fill" => {
                app.prose_width = None;
                app.push_system("Prose now **fills the pane**.");
            }
            "comfortable" | "read" => {
                app.prose_width = Some(crate::tui_text::PROSE_MEASURE as u16);
                app.push_system(format!(
                    "Prose capped at **{} columns** — easier to read, with space to the right.",
                    crate::tui_text::PROSE_MEASURE
                ));
            }
            other => match other.parse::<u16>() {
                Ok(columns) if columns >= 20 => {
                    app.prose_width = Some(columns);
                    app.push_system(format!("Prose capped at **{columns} columns**."));
                }
                Ok(_) => app.push_system("A measure below 20 columns is unreadable."),
                Err(_) => app.push_system(
                    "Usage: `/width full`, `/width comfortable`, or `/width <columns>`.",
                ),
            },
        }
        return false;
    }
    if raw == "/activity" || raw.starts_with("/activity ") {
        let argument = raw.strip_prefix("/activity").unwrap_or_default().trim();
        let requested = if argument.is_empty() {
            Some(app.activity_visibility.next())
        } else {
            ActivityVisibility::parse(argument)
        };
        match requested {
            Some(visibility) => {
                app.activity_visibility = visibility;
                let explanation = match visibility {
                    ActivityVisibility::Show => "pinned open.",
                    ActivityVisibility::AutoHide => {
                        "hidden while a turn runs, revealed when it finishes."
                    }
                    ActivityVisibility::Off => "hidden; the transcript takes the full width.",
                };
                app.push_system(format!(
                    "Run history **{}** — {explanation}",
                    visibility.label()
                ));
            }
            None => app
                .push_system("Usage: `/activity show`, `/activity autohide`, or `/activity off`."),
        }
        return false;
    }
    if raw == "/keys" {
        // Generated from the same table the key handler dispatches through, and
        // filtered to what this terminal can actually send, so it cannot
        // describe a chord that does nothing here.
        let mut out = String::from("Keyboard shortcuts\n");
        for (category, rows) in app.keys.help_lines() {
            out.push_str(&format!("\n**{}**\n", category.label()));
            for (chords, description) in rows {
                out.push_str(&format!("- `{chords}` — {description}\n"));
            }
        }
        if app.keys.terminal() == crate::tui_keys::TerminalKind::AppleTerminal {
            out.push_str(
                "\nApple Terminal does not forward `Home`, `End`, or modified \
                 arrow keys, and strips `Shift` from `PageUp`/`PageDown`, so the \
                 `Ctrl`+letter forms above are the ones that reach the \
                 workspace. Another terminal — iTerm2, Ghostty, WezTerm, kitty — \
                 delivers the full set.\n",
            );
        }
        app.push_system(out);
        return false;
    }
    if raw == "/mouse" || raw.starts_with("/mouse ") {
        let argument = raw.strip_prefix("/mouse").unwrap_or_default().trim();
        if let Some(value) = argument.strip_prefix("speed") {
            match value.trim().parse::<usize>() {
                Ok(lines) if (1..=20).contains(&lines) => {
                    app.wheel_lines = lines;
                    app.push_system(format!(
                        "Wheel step **{lines}** {} per notch.",
                        if lines == 1 { "line" } else { "lines" }
                    ));
                }
                _ => app.push_system(
                    "Usage: `/mouse speed <1-20>`. Three matches `vim`; raise it if \
                     your terminal sends one event per notch, lower it if the \
                     terminal already accelerates the wheel.",
                ),
            }
            return false;
        }
        if !argument.is_empty() {
            app.push_system("Usage: `/mouse` to toggle the wheel, or `/mouse speed <1-20>`.");
            return false;
        }
        app.toggle_mouse_capture();
        return false;
    }
    if raw == "/export" || raw.starts_with("/export ") {
        let path = raw.strip_prefix("/export").unwrap_or_default();
        match export_transcript(app, cfg, path.trim()) {
            Ok(path) => {
                app.push_system(format!("Exported the transcript to `{}`.", path.display()))
            }
            Err(error) => app.push_system(format!("Transcript export failed: {error}")),
        }
        return false;
    }

    match parse_chat_command(&raw) {
        ParsedChatCommand::Command(command) => {
            return dispatch_tui_command(
                command,
                app,
                runner,
                cfg,
                provider,
                model_name,
                session_service,
                checkpoint_store,
                runtime_tools.as_ref(),
                confirmation,
                &telemetry,
                &tx,
            )
            .await;
        }
        ParsedChatCommand::MissingArgument { usage } => {
            app.push_system(format!("Missing argument. Usage: `{usage}`"));
            return false;
        }
        ParsedChatCommand::UnknownCommand(command) => {
            match crate::skills::expand_skill_command(&raw) {
                Ok(Some(expanded)) => raw = expanded,
                Ok(None) => {
                    app.push_system(format!(
                        "Unknown command `{command}`. Press `Ctrl+P` to search all actions."
                    ));
                    return false;
                }
                Err(error) => {
                    app.push_system(format!("Skill discovery failed: {error}"));
                    return false;
                }
            }
        }
        ParsedChatCommand::NotACommand => {}
    }

    app.push_message(Message::new("YOU", raw.clone()));
    app.activities.clear();
    app.scroll_to_end();
    app.busy = true;
    app.active_agent = "starting".into();
    telemetry.emit_content(
        "chat.prompt",
        serde_json::json!({
            "content": raw,
            "mode": app.mode.label().to_ascii_lowercase(),
        }),
    );
    let prompt = if app.mode == Mode::Plan {
        format!(
            "Planning mode. Use plan_work when it materially helps. Inspect and explain only; do not modify files.\n\n{raw}"
        )
    } else {
        raw
    };
    let run_cfg = cfg.clone();
    let run_runner = runner.clone();
    let task = tokio::spawn(async move {
        if let Err(error) = enforce_prompt_limit(&prompt, run_cfg.max_prompt_chars) {
            let _ = tx.send(UiEvent::Error(error.to_string()));
            let _ = tx.send(UiEvent::Completed(String::new()));
            return;
        }
        let prompt = match apply_guardrail(
            &run_cfg,
            &telemetry,
            "input",
            run_cfg.guardrail_input_mode,
            &prompt,
        ) {
            Ok(prompt) => prompt,
            Err(error) => {
                let _ = tx.send(UiEvent::Error(error.to_string()));
                let _ = tx.send(UiEvent::Completed(String::new()));
                return;
            }
        };
        let policy = RetrievalPolicy {
            max_chunks: run_cfg.retrieval_max_chunks,
            max_chars: run_cfg.retrieval_max_chars,
            min_score: run_cfg.retrieval_min_score,
        };
        let prompt =
            augment_prompt_with_retrieval(retrieval.as_ref(), &prompt, policy).unwrap_or(prompt);
        let result = if buffered_output_required(run_cfg.guardrail_output_mode) {
            match run_prompt(&run_runner, &run_cfg, &prompt, &telemetry).await {
                Ok(answer) => apply_guardrail(
                    &run_cfg,
                    &telemetry,
                    "output",
                    run_cfg.guardrail_output_mode,
                    &answer,
                )
                .map(|answer| {
                    let _ = tx.send(UiEvent::TextDelta {
                        author: "zavora".into(),
                        text: answer.clone(),
                    });
                    let _ = tx.send(UiEvent::Completed(answer));
                }),
                Err(error) => Err(error),
            }
        } else {
            run_prompt_to_ui(&run_runner, &run_cfg, &prompt, &telemetry, tx.clone())
                .await
                .map(|_| ())
        };
        if let Err(error) = result {
            let _ = tx.send(UiEvent::Error(error.to_string()));
            let _ = tx.send(UiEvent::Completed(String::new()));
        }
    });
    app.task_abort = Some(task.abort_handle());
    false
}

#[allow(clippy::too_many_arguments)]
async fn dispatch_tui_command(
    command: ChatCommand,
    app: &mut App,
    runner: &mut Arc<Runner>,
    cfg: &mut RuntimeConfig,
    provider: &mut crate::cli::Provider,
    model_name: &mut String,
    session_service: &Arc<dyn adk_session::SessionService>,
    checkpoint_store: &mut CheckpointStore,
    runtime_tools: &ResolvedRuntimeTools,
    confirmation: &ToolConfirmationSettings,
    telemetry: &TelemetrySink,
    tx: &tokio::sync::mpsc::UnboundedSender<UiEvent>,
) -> bool {
    match command {
        ChatCommand::Exit => return true,
        ChatCommand::Help => app.palette = Some(PaletteState::default()),
        ChatCommand::Status => app.push_system(format_status_markdown(cfg, app)),
        ChatCommand::Tools => app.push_system(format_tools_markdown(cfg, runtime_tools)),
        ChatCommand::Mcp => app.push_system(format_mcp_markdown(cfg, runtime_tools)),
        ChatCommand::Capabilities => {
            let configured = cfg
                .mcp_servers
                .iter()
                .map(|server| server.name.clone())
                .collect::<Vec<_>>();
            app.push_system(crate::capabilities::format_catalog_markdown_with_runtime(
                &configured,
                &runtime_tools
                    .mcp_tool_names()
                    .iter()
                    .cloned()
                    .collect::<Vec<_>>(),
            ));
        }
        ChatCommand::Skills => app.push_system(crate::skills::format_skills_markdown()),
        ChatCommand::Plugins => match crate::plugins::format_plugins_markdown() {
            Ok(markdown) => app.push_system(markdown),
            Err(error) => app.push_system(format!("Plugin inspection failed: {error}")),
        },
        ChatCommand::Instructions(subcommand) => app.push_system(
            crate::skills::format_instructions_markdown(subcommand.trim() == "show"),
        ),
        ChatCommand::Agents => app.push_system(format_agents_markdown()),
        ChatCommand::Inspect => app.push_system(format_inspect_markdown(cfg, runtime_tools)),
        ChatCommand::Doctor => app.push_system(format_mcp_doctor_markdown(cfg, runtime_tools)),
        ChatCommand::Models => app.push_system(format_models_markdown(cfg)),
        ChatCommand::Usage => match snapshot_session_events(session_service, cfg).await {
            Ok(events) => {
                let usage =
                    compute_context_usage(&events, &provider.to_string(), &cfg.worker_model);
                app.push_system(format_context_markdown(&usage));
            }
            Err(error) => app.push_system(format!("Context inspection failed: {error}")),
        },
        ChatCommand::Sessions(subcommand) => {
            handle_sessions_command(app, cfg, session_service, &subcommand).await;
        }
        ChatCommand::NewSession(session_id) => {
            let session_id = if session_id.trim().is_empty() {
                format!(
                    "session-{}",
                    std::time::SystemTime::now()
                        .duration_since(std::time::UNIX_EPOCH)
                        .unwrap_or_default()
                        .as_millis()
                )
            } else {
                session_id.trim().to_string()
            };
            cfg.session_id = session_id.clone();
            match crate::session::ensure_session_exists(session_service, cfg).await {
                Ok(()) => {
                    app.messages.clear();
                    app.activities.clear();
                    app.context_percent = 0;
                    app.push_system(format!("Started session `{session_id}`."));
                }
                Err(error) => app.push_system(format!("Could not create session: {error}")),
            }
        }
        ChatCommand::Compact => {
            app.active_agent = "compacting".into();
            match crate::compact::compact_session(
                session_service,
                cfg,
                &crate::compact::CompactStrategy::default(),
            )
            .await
            {
                Ok(Some(summary)) => app.push_system(format!(
                    "## Conversation compacted\n\n{}",
                    crate::text::truncate(&summary, 2000, "…")
                )),
                Ok(None) => app.push_system("Conversation is too short to compact."),
                Err(error) => app.push_system(format!("Compaction failed: {error}")),
            }
            app.active_agent = "idle".into();
        }
        ChatCommand::Checkpoint(subcommand) => {
            handle_checkpoint_command(app, cfg, session_service, checkpoint_store, &subcommand)
                .await;
        }
        ChatCommand::Tangent(subcommand) => {
            handle_tangent_command(app, cfg, session_service, checkpoint_store, &subcommand).await;
        }
        ChatCommand::Todos(subcommand) => handle_todos_command(app, &subcommand),
        ChatCommand::Undo => match crate::file_history::undo_last() {
            Ok(message) => app.push_system(format!("✓ {message}")),
            Err(error) => app.push_system(format!("Cannot undo: {error}")),
        },
        ChatCommand::Allow(pattern) => {
            crate::tools::confirming::trust_tool(&pattern);
            app.push_system(format!("Session rule added: always allow `{pattern}`."));
        }
        ChatCommand::Deny(pattern) => {
            crate::tools::confirming::deny_tool(&pattern);
            app.push_system(format!(
                "Session rule added: always deny `{pattern}`. Deny rules override allow rules."
            ));
        }
        ChatCommand::Agent => {
            if crate::tools::confirming::is_agent_mode() {
                app.push_system("Agent mode is already active.");
            } else {
                app.approval = Some(PendingApproval {
                    tool: "agent mode".into(),
                    detail:
                        "Trust fs_read, fs_write, and execute_bash for the rest of this session?"
                            .into(),
                    response: None,
                    enables_agent_mode: true,
                });
            }
        }
        ChatCommand::AutoCompact => {
            cfg.auto_compact_enabled = !cfg.auto_compact_enabled;
            app.push_system(format!(
                "Auto-compaction {} at {:.0}% utilization (target {:.0}%).",
                if cfg.auto_compact_enabled {
                    "enabled"
                } else {
                    "disabled"
                },
                cfg.compaction_threshold * 100.0,
                cfg.compaction_target * 100.0
            ));
        }
        ChatCommand::Provider(provider_name) => {
            let next_provider = match crate::provider::parse_provider_name(&provider_name) {
                Ok(provider) => provider,
                Err(error) => {
                    app.push_system(format!("Provider switch failed: {error}"));
                    return false;
                }
            };
            let mut switched = cfg.clone();
            switched.provider = next_provider;
            switched.worker_provider = next_provider;
            switched.worker_model = crate::model_catalog::default_model(
                next_provider,
                crate::model_catalog::ModelRole::Worker,
            )
            .to_string();
            switched.model = Some(switched.worker_model.clone());
            switch_runtime(
                app,
                runner,
                cfg,
                provider,
                model_name,
                session_service,
                runtime_tools,
                confirmation,
                telemetry,
                switched,
                "worker provider",
            )
            .await;
        }
        ChatCommand::Model(next_model) | ChatCommand::Worker(next_model) => {
            let Some(next_model) = next_model else {
                app.push_system(format_models_markdown(cfg));
                return false;
            };
            let mut switched = cfg.clone();
            switched.model = Some(next_model.clone());
            switched.worker_model = next_model;
            switch_runtime(
                app,
                runner,
                cfg,
                provider,
                model_name,
                session_service,
                runtime_tools,
                confirmation,
                telemetry,
                switched,
                "worker model",
            )
            .await;
        }
        ChatCommand::PlannerProvider(provider_name) => {
            let next_provider = match crate::provider::parse_provider_name(&provider_name) {
                Ok(crate::cli::Provider::Auto) => {
                    app.push_system("Planner provider must be explicit.");
                    return false;
                }
                Ok(provider) => provider,
                Err(error) => {
                    app.push_system(format!("Planner switch failed: {error}"));
                    return false;
                }
            };
            let mut switched = cfg.clone();
            switched.planner_provider = next_provider;
            switched.planner_model = crate::model_catalog::default_model(
                next_provider,
                crate::model_catalog::ModelRole::Planner,
            )
            .to_string();
            switch_runtime(
                app,
                runner,
                cfg,
                provider,
                model_name,
                session_service,
                runtime_tools,
                confirmation,
                telemetry,
                switched,
                "planner provider",
            )
            .await;
        }
        ChatCommand::Planner(next_model) => {
            let Some(next_model) = next_model else {
                app.push_system(format_models_markdown(cfg));
                return false;
            };
            let mut switched = cfg.clone();
            switched.planner_model = next_model;
            switch_runtime(
                app,
                runner,
                cfg,
                provider,
                model_name,
                session_service,
                runtime_tools,
                confirmation,
                telemetry,
                switched,
                "planner model",
            )
            .await;
        }
        ChatCommand::Time(query) => {
            if query.trim().is_empty() {
                let context = crate::agents::time::TimeAgent::handshake();
                app.push_system(format!(
                    "## Time\n\n- **Current:** `{}`\n- **Timezone:** `{}`\n- **Weekday:** {}",
                    context.now_iso, context.timezone, context.weekday
                ));
            } else {
                match crate::agents::time::TimeAgent::parse_relative(&query) {
                    Ok(time) => app.push_system(format!("`{query}` → `{}`", time.to_rfc3339())),
                    Err(error) => app.push_system(format!("Time parsing failed: {error}")),
                }
            }
        }
        ChatCommand::Memory(subcommand) => handle_memory_command(app, &subcommand).await,
        ChatCommand::Delegate(task) => {
            if task.trim().is_empty() {
                app.push_system("Usage: `/delegate <task description>`.");
            } else {
                app.busy = true;
                app.active_agent = "delegate".into();
                let cfg = cfg.clone();
                let tools = runtime_tools.clone();
                let confirmation = confirmation.clone();
                let telemetry = telemetry.clone();
                let sessions = session_service.clone();
                let tx = tx.clone();
                let task = tokio::spawn(async move {
                    let result = crate::todos::run_delegate(
                        task.trim(),
                        &cfg,
                        sessions,
                        &tools,
                        &confirmation,
                        &telemetry,
                    )
                    .await;
                    let _ = tx.send(UiEvent::System(result.format_display()));
                    let _ = tx.send(UiEvent::Completed(String::new()));
                });
                app.task_abort = Some(task.abort_handle());
            }
        }
        ChatCommand::Ralph(prompt) => {
            if prompt.trim().is_empty() {
                app.push_system("Usage: `/ralph <goal>`.");
            } else {
                app.busy = true;
                app.active_agent = "ralph".into();
                let cfg = cfg.clone();
                let telemetry = telemetry.clone();
                let tx = tx.clone();
                let task = tokio::task::spawn_local(async move {
                    let retrieval = crate::retrieval::DisabledRetrievalService;
                    let message = match crate::ralph::run_ralph(
                        &cfg,
                        prompt,
                        crate::ralph::RalphRunOptions {
                            phase: None,
                            resume: false,
                            output_dir: None,
                            output_format: crate::cli::OutputFormat::Text,
                            always_approve: false,
                        },
                        &telemetry,
                        &retrieval,
                    )
                    .await
                    {
                        Ok(()) => "Ralph pipeline completed.".to_string(),
                        Err(error) => format!("Ralph pipeline failed: {error}"),
                    };
                    let _ = tx.send(UiEvent::System(message));
                    let _ = tx.send(UiEvent::Completed(String::new()));
                });
                app.task_abort = Some(task.abort_handle());
            }
        }
    }
    false
}

#[allow(clippy::too_many_arguments)]
async fn switch_runtime(
    app: &mut App,
    runner: &mut Arc<Runner>,
    cfg: &mut RuntimeConfig,
    provider: &mut crate::cli::Provider,
    model_name: &mut String,
    session_service: &Arc<dyn adk_session::SessionService>,
    runtime_tools: &ResolvedRuntimeTools,
    confirmation: &ToolConfirmationSettings,
    telemetry: &TelemetrySink,
    switched: RuntimeConfig,
    route: &str,
) {
    app.active_agent = "switching route".into();
    match build_single_runner_for_chat(
        &switched,
        session_service.clone(),
        runtime_tools,
        confirmation,
        telemetry,
    )
    .await
    {
        Ok((next_runner, next_provider, next_model)) => {
            *runner = Arc::new(next_runner);
            *provider = next_provider;
            *model_name = next_model.clone();
            *cfg = switched;
            cfg.provider = next_provider;
            cfg.model = Some(next_model.clone());
            cfg.worker_provider = next_provider;
            cfg.worker_model = next_model;
            app.push_system(format!(
                "Switched {route}.\n\n- **Worker:** {} / `{}`\n- **Planner:** {} / `{}`\n- **Session:** preserved",
                cfg.worker_provider, cfg.worker_model, cfg.planner_provider, cfg.planner_model
            ));
        }
        Err(error) => app.push_system(format!(
            "Could not switch {route}; the previous route remains active.\n\n`{}`",
            crate::error::format_cli_error(&error, cfg.show_sensitive_config)
        )),
    }
    app.active_agent = "idle".into();
}

async fn handle_checkpoint_command(
    app: &mut App,
    cfg: &RuntimeConfig,
    session_service: &Arc<dyn adk_session::SessionService>,
    store: &mut CheckpointStore,
    subcommand: &str,
) {
    let parts = subcommand.split_whitespace().collect::<Vec<_>>();
    match parts.first().copied() {
        Some("save") => match snapshot_session_events(session_service, cfg).await {
            Ok(events) => {
                let label = parts
                    .get(1..)
                    .map(|parts| parts.join(" "))
                    .unwrap_or_default();
                let checkpoint = store.save(&label, events).clone();
                let workspace = std::env::current_dir().unwrap_or_default();
                match store.save_to_disk(&workspace) {
                    Ok(()) => app.push_system(format!(
                        "Saved checkpoint **[{}] {}** with {} events.",
                        checkpoint.tag,
                        checkpoint.label,
                        checkpoint.events.len()
                    )),
                    Err(error) => app.push_system(format!(
                        "Checkpoint was created in memory but could not be persisted: {error}"
                    )),
                }
            }
            Err(error) => app.push_system(format!("Checkpoint failed: {error}")),
        },
        Some("list") | None => app.push_system(format_checkpoint_list(store)),
        Some("restore") => {
            let Some(tag) = parts.get(1).and_then(|tag| tag.parse::<usize>().ok()) else {
                app.push_system("Usage: `/checkpoint restore TAG`.");
                return;
            };
            let Some(events) = store.get(tag).map(|checkpoint| checkpoint.events.clone()) else {
                app.push_system(format!("No checkpoint with tag `{tag}`."));
                return;
            };
            match restore_session_events(session_service, cfg, &events).await {
                Ok(()) => {
                    app.messages.clear();
                    app.activities.clear();
                    app.push_system(format!(
                        "Restored checkpoint **[{tag}]**. The visible transcript was cleared to match the restored session."
                    ));
                }
                Err(error) => app.push_system(format!("Checkpoint restore failed: {error}")),
            }
        }
        _ => app.push_system(
            "Usage: `/checkpoint save [label]`, `/checkpoint list`, or `/checkpoint restore TAG`.",
        ),
    }
}

async fn handle_sessions_command(
    app: &mut App,
    cfg: &mut RuntimeConfig,
    session_service: &Arc<dyn adk_session::SessionService>,
    subcommand: &str,
) {
    let parts = subcommand.split_whitespace().collect::<Vec<_>>();
    let requested = match parts.as_slice() {
        [] | ["list"] => None,
        ["switch", session_id, ..] => Some(*session_id),
        [session_id, ..] => Some(*session_id),
    };
    if let Some(session_id) = requested {
        match session_service
            .get(adk_session::GetRequest {
                app_name: cfg.app_name.clone(),
                user_id: cfg.user_id.clone(),
                session_id: session_id.to_string(),
                num_recent_events: None,
                after: None,
            })
            .await
        {
            Ok(session) => {
                cfg.session_id = session_id.to_string();
                app.messages = session
                    .events()
                    .all()
                    .into_iter()
                    .filter_map(|event| {
                        let text = crate::streaming::event_text(&event);
                        (!text.trim().is_empty()).then(|| {
                            let role = if event.author == "user" {
                                "YOU".to_string()
                            } else {
                                event.author.to_ascii_uppercase()
                            };
                            Message::new(role, text)
                        })
                    })
                    .collect();
                app.activities.clear();
                app.scroll_to_end();
                app.current_assistant = None;
                app.push_system(format!("Switched to session `{session_id}`."));
            }
            Err(error) => app.push_system(format!(
                "Could not switch to session `{session_id}`: {error}"
            )),
        }
        return;
    }

    match session_service
        .list(adk_session::ListRequest {
            app_name: cfg.app_name.clone(),
            user_id: cfg.user_id.clone(),
            limit: None,
            offset: None,
        })
        .await
    {
        Ok(mut sessions) => {
            sessions.sort_by_key(|session| std::cmp::Reverse(session.last_update_time()));
            let mut output = String::from("## Sessions\n\n");
            if sessions.is_empty() {
                output.push_str("No persisted sessions were found.");
            } else {
                for session in sessions {
                    output.push_str(&format!(
                        "- {} `{}` — {}\n",
                        if session.id() == cfg.session_id {
                            "**active**"
                        } else {
                            "available"
                        },
                        session.id(),
                        session.last_update_time().to_rfc3339()
                    ));
                }
                output.push_str("\nUse `/sessions switch ID` to load one.");
            }
            app.push_system(output);
        }
        Err(error) => app.push_system(format!("Session listing failed: {error}")),
    }
}

/// Put text on the system clipboard.
///
/// The full-screen workspace claims mouse reporting so the wheel can scroll,
/// which takes the terminal's own click-drag selection away. Rather than force a
/// choice between scrolling and copying, the app can hand text over directly.
///
/// Uses the platform tool rather than a clipboard crate: no new dependency, and
/// it works over SSH where a linked X11/Wayland clipboard would not.
fn copy_to_clipboard(text: &str) -> anyhow::Result<&'static str> {
    use std::io::Write;
    use std::process::{Command, Stdio};

    // Ordered by platform likelihood; the first present binary wins.
    let candidates: &[(&str, &[&str])] = if cfg!(target_os = "macos") {
        &[("pbcopy", &[])]
    } else if cfg!(target_os = "windows") {
        &[("clip", &[])]
    } else {
        &[
            ("wl-copy", &[]),
            ("xclip", &["-selection", "clipboard"]),
            ("xsel", &["--clipboard", "--input"]),
        ]
    };

    for (binary, args) in candidates {
        let Ok(mut child) = Command::new(binary)
            .args(*args)
            .stdin(Stdio::piped())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
        else {
            continue; // not installed; try the next
        };

        if let Some(stdin) = child.stdin.as_mut() {
            stdin
                .write_all(text.as_bytes())
                .with_context(|| format!("failed to write to {binary}"))?;
        }
        let status = child
            .wait()
            .with_context(|| format!("failed to run {binary}"))?;
        if status.success() {
            return Ok(binary);
        }
    }

    anyhow::bail!(
        "no clipboard tool available (looked for {}). Use `/export` to write the transcript to a file instead.",
        candidates
            .iter()
            .map(|(binary, _)| *binary)
            .collect::<Vec<_>>()
            .join(", ")
    )
}

/// The text `/copy` should place on the clipboard.
///
/// Defaults to the most recent assistant response, which is what a developer
/// almost always wants: the code or answer just produced. `all` takes the whole
/// transcript.
fn clipboard_payload(app: &App, whole_transcript: bool) -> Option<String> {
    if whole_transcript {
        let body = app
            .messages
            .iter()
            .filter(|message| message.role != ELIDED_ROLE)
            .map(|message| format!("## {}\n\n{}", message.role, message.text))
            .collect::<Vec<_>>()
            .join("\n\n");
        return (!body.trim().is_empty()).then_some(body);
    }

    app.messages
        .iter()
        .rev()
        .find(|message| message.role != "YOU" && message.role != ELIDED_ROLE)
        .map(|message| message.text.clone())
        .filter(|text| !text.trim().is_empty())
}

fn export_transcript(
    app: &App,
    cfg: &RuntimeConfig,
    requested_path: &str,
) -> Result<std::path::PathBuf> {
    let path = if requested_path.is_empty() {
        std::path::PathBuf::from(".zavora")
            .join("exports")
            .join(format!("{}.md", cfg.session_id))
    } else {
        std::path::PathBuf::from(requested_path)
    };
    if let Some(parent) = path.parent()
        && !parent.as_os_str().is_empty()
    {
        std::fs::create_dir_all(parent)
            .with_context(|| format!("failed to create export directory '{}'", parent.display()))?;
    }
    let markdown = format_transcript_markdown(
        app,
        &cfg.session_id,
        &cfg.profile,
        &cfg.worker_provider.to_string(),
        &cfg.worker_model,
    );
    std::fs::write(&path, markdown)
        .with_context(|| format!("failed to write transcript '{}'", path.display()))?;
    Ok(path)
}

fn format_transcript_markdown(
    app: &App,
    session_id: &str,
    profile: &str,
    provider: &str,
    model: &str,
) -> String {
    let mut markdown = format!(
        "# Zavora session {}\n\nProfile: `{}`  \nWorker: {} / `{}`\n\n",
        session_id, profile, provider, model
    );
    for message in &app.messages {
        markdown.push_str(&format!("## {}\n\n{}\n\n", message.role, message.text));
    }
    markdown
}

async fn handle_tangent_command(
    app: &mut App,
    cfg: &RuntimeConfig,
    session_service: &Arc<dyn adk_session::SessionService>,
    store: &mut CheckpointStore,
    subcommand: &str,
) {
    if store.in_tangent() {
        let events = if subcommand.trim() == "tail" {
            match snapshot_session_events(session_service, cfg).await {
                Ok(current) => store.exit_tangent_tail(&current),
                Err(error) => {
                    app.push_system(format!("Could not inspect tangent session: {error}"));
                    return;
                }
            }
        } else {
            store.exit_tangent()
        };
        let Some(events) = events else {
            app.push_system("The tangent baseline is unavailable.");
            return;
        };
        match restore_session_events(session_service, cfg, &events).await {
            Ok(()) => {
                app.messages.clear();
                app.activities.clear();
                let workspace = std::env::current_dir().unwrap_or_default();
                if let Err(error) = store.save_to_disk(&workspace) {
                    app.push_system(format!(
                        "Tangent state changed but could not be persisted: {error}"
                    ));
                }
                app.push_system(if subcommand.trim() == "tail" {
                    "Exited tangent mode and retained the latest exchange."
                } else {
                    "Exited tangent mode and restored the baseline conversation."
                });
            }
            Err(error) => app.push_system(format!("Tangent restore failed: {error}")),
        }
    } else {
        match snapshot_session_events(session_service, cfg).await {
            Ok(events) => {
                let tag = store.enter_tangent(events);
                let workspace = std::env::current_dir().unwrap_or_default();
                if let Err(error) = store.save_to_disk(&workspace) {
                    app.push_system(format!(
                        "Entered tangent mode at [{tag}], but persistence failed: {error}"
                    ));
                } else {
                    app.push_system(format!(
                        "Entered tangent mode at checkpoint **[{tag}]**. Use `/tangent` to discard the branch or `/tangent tail` to retain its latest exchange."
                    ));
                }
            }
            Err(error) => app.push_system(format!("Could not enter tangent mode: {error}")),
        }
    }
}

fn handle_todos_command(app: &mut App, subcommand: &str) {
    let workspace = std::env::current_dir().unwrap_or_default();
    let parts = subcommand.split_whitespace().collect::<Vec<_>>();
    let result = match parts.first().copied() {
        Some("view") => parts
            .get(1)
            .ok_or_else(|| anyhow::anyhow!("Usage: /todos view ID"))
            .and_then(|id| crate::todos::load_todo(&workspace, id))
            .map(|todo| todo.format_display()),
        Some("delete") => parts
            .get(1)
            .ok_or_else(|| anyhow::anyhow!("Usage: /todos delete ID"))
            .and_then(|id| {
                crate::todos::delete_todo(&workspace, id)?;
                Ok(format!("Deleted todo `{id}`."))
            }),
        Some("clear-finished") => crate::todos::clear_finished_todos(&workspace)
            .map(|count| format!("Cleared {count} finished todo list(s).")),
        _ => crate::todos::format_todos_summary(&workspace),
    };
    match result {
        Ok(output) => app.push_system(output),
        Err(error) => app.push_system(format!("Todo command failed: {error}")),
    }
}

async fn handle_memory_command(app: &mut App, subcommand: &str) {
    let parts = subcommand.split_whitespace().collect::<Vec<_>>();
    let argument = parts
        .get(1..)
        .map(|parts| parts.join(" "))
        .unwrap_or_default();
    match parts.first().copied() {
        Some("recall") => match crate::agents::memory::recall(&argument, 10).await {
            Ok(memories) if memories.is_empty() => {
                app.push_system(format!("No memories found for `{argument}`."));
            }
            Ok(memories) => app.push_system(format!(
                "## Recalled memories\n\n{}",
                memories
                    .iter()
                    .map(|memory| format!("- {memory}"))
                    .collect::<Vec<_>>()
                    .join("\n")
            )),
            Err(error) => app.push_system(format!("Memory recall failed: {error}")),
        },
        Some("remember") if !argument.is_empty() => {
            match crate::agents::memory::remember(&argument).await {
                Ok(()) => app.push_system("Memory stored."),
                Err(error) => app.push_system(format!("Memory storage failed: {error}")),
            }
        }
        Some("forget") if !argument.is_empty() => {
            match crate::agents::memory::forget(&argument).await {
                Ok(count) => app.push_system(format!("Removed {count} memories.")),
                Err(error) => app.push_system(format!("Memory removal failed: {error}")),
            }
        }
        _ => app.push_system("Usage: `/memory recall|remember|forget <text>`."),
    }
}

fn format_status_markdown(cfg: &RuntimeConfig, app: &App) -> String {
    format!(
        "## Workspace status\n\n- **Profile:** `{}`\n- **Agent:** `{}` ({})\n- **Mode:** {}\n- **Worker:** {} / `{}`\n- **Planner:** {} / `{}`\n- **Session:** `{}`\n- **Context:** {}%\n- **Auto-compact:** {}\n- **Run history:** {}\n- **Mouse capture:** {}",
        cfg.profile,
        cfg.agent_name,
        cfg.agent_source.label(),
        app.mode.label(),
        cfg.worker_provider,
        cfg.worker_model,
        cfg.planner_provider,
        cfg.planner_model,
        cfg.session_id,
        app.context_percent,
        if cfg.auto_compact_enabled {
            "on"
        } else {
            "off"
        },
        app.activity_visibility.label(),
        if app.mouse_capture {
            "on (wheel scrolls; `/mouse` to select text)"
        } else {
            "off (mouse selection available)"
        }
    )
}

fn format_tools_markdown(cfg: &RuntimeConfig, runtime_tools: &ResolvedRuntimeTools) -> String {
    let mut names = runtime_tools
        .tools()
        .iter()
        .map(|tool| tool.name().to_string())
        .collect::<Vec<_>>();
    names.sort();
    let mut output = format!(
        "## Runtime tools\n\n- **Available:** {}\n- **MCP tools:** {}\n- **Confirmation mode:** `{:?}`\n\n",
        names.len(),
        runtime_tools.mcp_tool_names().len(),
        cfg.tool_confirmation_mode
    );
    for name in names {
        let source = if runtime_tools.mcp_tool_names().contains(&name) {
            "MCP"
        } else {
            "built-in"
        };
        output.push_str(&format!("- `{name}` — {source}\n"));
    }
    output
}

fn format_inspect_markdown(cfg: &RuntimeConfig, runtime_tools: &ResolvedRuntimeTools) -> String {
    let configured = cfg
        .mcp_servers
        .iter()
        .map(|server| server.name.clone())
        .collect::<Vec<_>>();
    let instruction_status = match crate::skills::resolve_workspace_instructions() {
        Ok(instructions) => format!(
            "{} active / {} deferred",
            instructions.sources.len(),
            instructions.deferred_sources.len()
        ),
        Err(error) => format!("unavailable ({error})"),
    };
    format!(
        "## Runtime inspection\n\n- **Profile:** `{}`\n- **Agent:** `{}`\n- **Worker:** {} / `{}`\n- **Planner:** {} / `{}`\n- **Session backend:** `{:?}`\n- **MCP:** {} configured / {} connected tools\n- **Instructions:** {}\n- **Capabilities:** `{}`\n\n{}",
        cfg.profile,
        cfg.agent_name,
        cfg.worker_provider,
        cfg.worker_model,
        cfg.planner_provider,
        cfg.planner_model,
        cfg.session_backend,
        cfg.mcp_servers.len(),
        runtime_tools.mcp_tool_names().len(),
        instruction_status,
        crate::capabilities::state_path().display(),
        crate::capabilities::format_catalog_markdown_with_runtime(
            &configured,
            &runtime_tools
                .mcp_tool_names()
                .iter()
                .cloned()
                .collect::<Vec<_>>(),
        )
    )
}

fn format_models_markdown(cfg: &RuntimeConfig) -> String {
    let mut output = format!(
        "## Model routes\n\n- **Worker:** {} / `{}`\n- **Planner:** {} / `{}`\n\n",
        cfg.worker_provider, cfg.worker_model, cfg.planner_provider, cfg.planner_model
    );
    let models = crate::model_catalog::models_for_provider(cfg.worker_provider);
    if models.is_empty() {
        output.push_str("This provider accepts model identifiers directly. Use `/model MODEL`.\n");
    } else {
        output.push_str("### Selectable worker models\n\n");
        for model in models {
            output.push_str(&format!(
                "- `{}` — {} · {}\n",
                model.id,
                model.recommended_role.label(),
                model.description
            ));
        }
    }
    output
}

fn format_context_markdown(usage: &crate::context::ContextUsage) -> String {
    let total = usage.total_tokens();
    let remaining = usage.context_window_tokens.saturating_sub(total);
    format!(
        "## Context usage\n\n- **Used:** {total} / {} tokens ({})\n- **Remaining:** {remaining} tokens\n- **Events:** {}\n- **User:** ~{} tokens\n- **Assistant:** ~{} tokens\n- **Tools:** ~{} tokens",
        usage.context_window_tokens,
        usage.prompt_indicator(),
        usage.event_count,
        crate::context::estimate_tokens(usage.user_chars),
        crate::context::estimate_tokens(usage.assistant_chars),
        crate::context::estimate_tokens(usage.tool_chars)
    )
}

fn run_shell_command(
    app: &mut App,
    cfg: &RuntimeConfig,
    command: String,
    session_service: Arc<dyn adk_session::SessionService>,
    telemetry: TelemetrySink,
    tx: tokio::sync::mpsc::UnboundedSender<UiEvent>,
) {
    app.push_message(Message::new("YOU", format!("$ {command}")));
    app.activities.clear();
    app.activities.push(Activity {
        call_id: Some("direct-shell".into()),
        name: "execute_bash".into(),
        detail: command.clone(),
        state: ActivityState::Running,
        started: Instant::now(),
        elapsed: None,
    });
    app.busy = true;
    app.active_agent = "shell".into();
    telemetry.emit_content(
        "tui.shell.started",
        serde_json::json!({ "command": command }),
    );
    let session_id = cfg.session_id.clone();
    let task = tokio::spawn(async move {
        let shell = std::env::var("SHELL").unwrap_or_else(|_| "sh".into());
        let output = tokio::process::Command::new(&shell)
            .arg("-lc")
            .arg(&command)
            .current_dir(std::env::current_dir().unwrap_or_default())
            .output()
            .await;
        match output {
            Ok(output) => {
                let success = output.status.success();
                let stdout = String::from_utf8_lossy(&output.stdout);
                let stderr = String::from_utf8_lossy(&output.stderr);
                let combined = match (stdout.trim().is_empty(), stderr.trim().is_empty()) {
                    (false, false) => format!("{stdout}\n{stderr}"),
                    (false, true) => stdout.into_owned(),
                    (true, false) => stderr.into_owned(),
                    (true, true) => "(command produced no output)".into(),
                };
                let detail = format!(
                    "exit {}",
                    output
                        .status
                        .code()
                        .map_or_else(|| "signal".into(), |code| code.to_string())
                );
                let _ = tx.send(UiEvent::ToolFinished {
                    call_id: Some("direct-shell".into()),
                    name: "execute_bash".into(),
                    success,
                    detail,
                });
                let _ = tx.send(UiEvent::System(format!(
                    "## Shell output\n\n```text\n{}\n```",
                    crate::text::truncate(&combined, 50_000, "\n… output truncated")
                )));
                let mut event = Event::new("tui-shell");
                event.author = "user".into();
                event.llm_response.content = Some(Content::new("user").with_text(format!(
                    "Direct shell command: {command}\nExit status: {}\nOutput:\n{}",
                    output
                        .status
                        .code()
                        .map_or_else(|| "signal".into(), |code| code.to_string()),
                    crate::text::truncate(&combined, 20_000, "\n… output truncated")
                )));
                if let Err(error) = session_service.append_event(&session_id, event).await {
                    let _ = tx.send(UiEvent::System(format!(
                        "Shell output was shown but could not be added to agent context: {error}"
                    )));
                }
                telemetry.emit(
                    "tui.shell.completed",
                    serde_json::json!({
                        "session_id": session_id,
                        "success": success,
                        "exit_code": output.status.code(),
                    }),
                );
            }
            Err(error) => {
                let _ = tx.send(UiEvent::ToolFinished {
                    call_id: Some("direct-shell".into()),
                    name: "execute_bash".into(),
                    success: false,
                    detail: error.to_string(),
                });
                let _ = tx.send(UiEvent::System(format!("Shell command failed: {error}")));
            }
        }
        let _ = tx.send(UiEvent::Completed(String::new()));
    });
    app.task_abort = Some(task.abort_handle());
}

fn previous_boundary(text: &str, cursor: usize) -> usize {
    text[..cursor]
        .char_indices()
        .next_back()
        .map_or(0, |(index, _)| index)
}

fn next_boundary(text: &str, cursor: usize) -> usize {
    text[cursor..]
        .char_indices()
        .nth(1)
        .map_or(text.len(), |(index, _)| cursor + index)
}

fn previous_word_boundary(text: &str, cursor: usize) -> usize {
    let prefix = &text[..cursor];
    let trimmed = prefix.trim_end_matches(char::is_whitespace);
    trimmed
        .char_indices()
        .rev()
        .find(|(_, character)| character.is_whitespace())
        .map_or(0, |(index, character)| index + character.len_utf8())
}

fn next_word_boundary(text: &str, cursor: usize) -> usize {
    let suffix = &text[cursor..];
    let mut seen_word = false;
    for (offset, character) in suffix.char_indices() {
        if character.is_whitespace() {
            if seen_word {
                return cursor + offset;
            }
        } else {
            seen_word = true;
        }
    }
    text.len()
}

fn draw(frame: &mut Frame<'_>, app: &App, cfg: &RuntimeConfig) {
    let area = frame.area();
    let composer_height = composer_height(app, area.width, area.height);
    let root = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(2),
            Constraint::Min(8),
            Constraint::Length(composer_height),
            Constraint::Length(1),
        ])
        .split(area);
    draw_header(frame, root[0], app, cfg);
    let show_activity = app.show_activity();
    if area.width >= 110 && show_activity {
        let body = Layout::default()
            .direction(Direction::Horizontal)
            .constraints([Constraint::Percentage(70), Constraint::Percentage(30)])
            .split(root[1]);
        draw_transcript(frame, body[0], app);
        draw_activity(frame, body[1], app);
    } else if show_activity {
        let activity_height = if area.height < 22 { 4 } else { 7 };
        let body = Layout::default()
            .direction(Direction::Vertical)
            .constraints([Constraint::Min(5), Constraint::Length(activity_height)])
            .split(root[1]);
        draw_transcript(frame, body[0], app);
        draw_activity(frame, body[1], app);
    } else {
        // Hidden: the transcript takes the whole body, at any width.
        draw_transcript(frame, root[1], app);
    }
    draw_composer(frame, root[2], app);
    draw_footer(frame, root[3], app, cfg);
    if app.palette.is_none() && app.approval.is_none() {
        draw_command_suggestions(frame, root[2], app);
    }
    if app.palette.is_some() {
        draw_palette(frame, area, app);
    }
    if let Some(approval) = &app.approval {
        draw_approval(frame, area, approval);
    }
}

fn draw_header(frame: &mut Frame<'_>, area: Rect, app: &App, cfg: &RuntimeConfig) {
    let workspace = std::env::current_dir()
        .ok()
        .and_then(|p| p.file_name().map(|n| n.to_string_lossy().into_owned()))
        .unwrap_or_default();
    let worker = crate::text::truncate(&cfg.worker_model, 28, "…");
    let line = Line::from(vec![
        Span::styled(
            " ZAVORA ",
            Style::default()
                .fg(Color::Black)
                .bg(ORANGE)
                .add_modifier(Modifier::BOLD),
        ),
        Span::styled(
            format!("  {workspace}  "),
            Style::default().fg(TEXT).add_modifier(Modifier::BOLD),
        ),
        Span::styled(
            format!(
                " {} ",
                if app.shell_mode {
                    "SHELL"
                } else {
                    app.mode.label()
                }
            ),
            Style::default()
                .fg(Color::Black)
                .bg(if app.shell_mode {
                    Color::Yellow
                } else if app.mode == Mode::Plan {
                    Color::Cyan
                } else {
                    Color::Green
                })
                .add_modifier(Modifier::BOLD),
        ),
        Span::raw("  "),
        Span::styled(
            format!("{worker}  /  {}", app.active_agent),
            Style::default().fg(MUTED),
        ),
    ]);
    frame.render_widget(
        Paragraph::new(line)
            .block(
                Block::default()
                    .borders(Borders::BOTTOM)
                    .border_style(Style::default().fg(Color::DarkGray)),
            )
            .alignment(Alignment::Left),
        area,
    );
}

fn draw_transcript(frame: &mut Frame<'_>, area: Rect, app: &App) {
    use crate::tui_text::GUTTER;

    // Recorded even on the empty path so a page step is never a stale guess.
    app.viewport.set(area.height.saturating_sub(1) as usize);
    if app.messages.is_empty() {
        app.max_scroll.set(0);
        app.message_rows.borrow_mut().clear();
        draw_welcome(frame, area);
        return;
    }

    // Content width excludes the right border and the gutter on each side.
    let content_width = area.width.saturating_sub(1 + (GUTTER as u16) * 2).max(8) as usize;
    let gutter = " ".repeat(GUTTER);
    let measure = app.prose_measure(content_width);

    let mut lines: Vec<Line<'static>> = Vec::new();
    let mut message_rows: Vec<usize> = Vec::with_capacity(app.messages.len());
    for (index, message) in app.messages.iter().enumerate() {
        // Space separates messages instead of a rule. A full-width divider plus
        // a reverse-video badge per message is a lot of chrome; whitespace does
        // the same grouping work without competing with the content.
        if index > 0 {
            lines.push(Line::default());
        }

        // The blank separator belongs to the boundary: landing on it puts a
        // little air above the message rather than jamming it to the top row.
        message_rows.push(lines.len().saturating_sub(1));

        if message.role == ELIDED_ROLE {
            lines.push(Line::from(vec![
                Span::raw(gutter.clone()),
                Span::styled(
                    format!("⋯ {}", message.text),
                    Style::default().fg(MUTED).add_modifier(Modifier::ITALIC),
                ),
            ]));
            continue;
        }

        let (color, label) = match message.role.as_str() {
            "YOU" => (Color::Cyan, "you"),
            "ZAVORA" => (ORANGE, "zavora"),
            _ => (Color::Green, "agent"),
        };

        // A lowercase coloured label reads as a speaker attribution rather than
        // a UI chip, and costs one line instead of two.
        lines.push(Line::from(vec![
            Span::raw(gutter.clone()),
            Span::styled(
                label.to_string(),
                Style::default().fg(color).add_modifier(Modifier::BOLD),
            ),
        ]));

        for line in message.lines(measure) {
            let mut spans = Vec::with_capacity(line.spans.len() + 1);
            spans.push(Span::raw(gutter.clone()));
            spans.extend(line.spans);
            lines.push(Line::from(spans).style(line.style));
        }
    }

    // Lines arrive pre-wrapped, so height is a simple count. With no title the
    // block consumes only the bottom border row, so the viewport is height - 1.
    // Getting this wrong by one is why streamed output stopped a line short of
    // the bottom: an overestimated viewport makes max_scroll too small to reach
    // the final line.
    let visible = area.height.saturating_sub(1) as usize;
    let rendered_height = lines.len();
    let max_scroll = rendered_height.saturating_sub(visible);

    // Publish the bounds for the key handler. Recorded here because only the
    // renderer knows the wrapped line count and the pane height.
    app.max_scroll.set(max_scroll);
    app.viewport.set(visible);
    *app.message_rows.borrow_mut() = message_rows;

    // Following means pinning to the tail every frame; a detached view keeps the
    // offset it was given, so lines arriving below it do not move it.
    let offset = if app.follow_output {
        max_scroll
    } else {
        app.scroll_offset.min(max_scroll)
    };

    // No title on the top edge: the pane is self-evident, and a heading spends a
    // row of the viewport on chrome. The scroll indicator goes on the bottom
    // border, which is already chrome, so the viewport is the same height whether
    // or not it is showing. A top title cost a content row only while scrolled —
    // ratatui reserves a row for one even without a top border — so a one-line
    // scroll moved the view by two.
    let mut block = Block::default()
        .borders(Borders::RIGHT | Borders::BOTTOM)
        .border_style(Style::default().fg(Color::DarkGray));
    let below = max_scroll.saturating_sub(offset);
    if below > 0 {
        let resume = app
            .keys
            .advertised_key(crate::tui_keys::ActionId::ScrollToEnd)
            .map(|key| key.display())
            .unwrap_or_else(|| "/keys".into());
        block = block.title_bottom(format!(" {below} lines below · {resume} newest "));
    }

    // Take the visible window rather than handing the whole transcript to
    // `Paragraph::scroll`, whose offset is a `u16`: a long session renders past
    // 65535 rows, and there the offset saturated and the top of the conversation
    // became unreachable. Slicing also spares the widget walking the lines above
    // the viewport on every frame.
    let window: Vec<Line<'static>> = lines.into_iter().skip(offset).take(visible).collect();

    frame.render_widget(Paragraph::new(Text::from(window)).block(block), area);
}

fn draw_welcome(frame: &mut Frame<'_>, area: Rect) {
    let height = 12.min(area.height);
    // Flush to the top of the pane, not centred in it. Centring left a bank of
    // blank rows between the header and the first thing worth reading, and the
    // transcript itself starts at the top of this pane — so the welcome now sits
    // exactly where the first exchange will appear instead of jumping there.
    let welcome_area = Rect::new(
        area.x.saturating_add(2),
        area.y,
        area.width.saturating_sub(4),
        height,
    );
    let text = Text::from(vec![
        Line::from(Span::styled(
            "ZAVORA",
            Style::default().fg(ORANGE).add_modifier(Modifier::BOLD),
        )),
        Line::from(Span::styled(
            "A focused AI engineering workspace",
            Style::default().fg(TEXT).add_modifier(Modifier::BOLD),
        )),
        Line::default(),
        Line::from(Span::styled(
            "Describe an outcome, ask about the repository, or plan a change.",
            Style::default().fg(MUTED),
        )),
        Line::default(),
        Line::from(vec![
            Span::styled(
                "BUILD  ",
                Style::default()
                    .fg(Color::Green)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::styled("inspect, edit, run, and verify", Style::default().fg(TEXT)),
        ]),
        Line::from(vec![
            Span::styled(
                "PLAN   ",
                Style::default()
                    .fg(Color::Cyan)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::styled(
                "reason through the work without changing files",
                Style::default().fg(TEXT),
            ),
        ]),
        Line::default(),
        Line::from(Span::styled(
            "Shift+Tab mode · Ctrl+P actions · /keys shortcuts · ! shell · /copy clipboard · /mouse select",
            Style::default().fg(MUTED),
        )),
    ]);
    frame.render_widget(
        Paragraph::new(text).alignment(Alignment::Center),
        welcome_area,
    );
}

fn draw_activity(frame: &mut Frame<'_>, area: Rect, app: &App) {
    let items = if app.activities.is_empty() {
        vec![ListItem::new(Line::from(Span::styled(
            "The agent's tools and results will stay here.",
            Style::default().fg(MUTED),
        )))]
    } else {
        app.activities
            .iter()
            .rev()
            .take((area.height.saturating_sub(2) / 3).max(1) as usize)
            .map(|item| {
                let (icon, color) = match item.state {
                    ActivityState::Running => ("◆", Color::Yellow),
                    ActivityState::Passed => ("✓", Color::Green),
                    ActivityState::Failed => ("×", Color::Red),
                };
                let elapsed = item.elapsed.unwrap_or_else(|| item.started.elapsed());
                let duration = if elapsed.as_secs() > 0 {
                    format!(
                        "{}.{:01}s",
                        elapsed.as_secs(),
                        elapsed.subsec_millis() / 100
                    )
                } else {
                    format!("{}ms", elapsed.as_millis())
                };
                ListItem::new(vec![
                    Line::from(vec![
                        Span::styled(format!("{icon} "), Style::default().fg(color)),
                        Span::styled(
                            friendly_tool_name(&item.name),
                            Style::default().fg(TEXT).add_modifier(Modifier::BOLD),
                        ),
                        Span::styled(format!("  {duration}"), Style::default().fg(MUTED)),
                    ]),
                    Line::from(Span::styled(
                        friendly_detail(&item.detail, area.width.saturating_sub(4) as usize),
                        Style::default().fg(MUTED),
                    )),
                    Line::default(),
                ])
            })
            .collect()
    };
    frame.render_widget(
        List::new(items)
            .block(
                Block::default()
                    .title(if app.busy {
                        " Live run "
                    } else {
                        " Run history "
                    })
                    .borders(Borders::LEFT | Borders::BOTTOM)
                    .border_style(Style::default().fg(Color::DarkGray)),
            )
            .style(Style::default().bg(PANEL)),
        area,
    );
}

fn draw_composer(frame: &mut Frame<'_>, area: Rect, app: &App) {
    let title = if app.busy {
        " Running  ·  Esc requests cancellation "
    } else if app.shell_mode {
        " Direct shell  ·  Esc exits shell mode "
    } else {
        " Ask Zavora "
    };
    let content = if app.input.is_empty() {
        Text::styled(
            if app.shell_mode {
                "$  Enter a shell command…"
            } else {
                "›  Describe the result you want…"
            },
            Style::default().fg(MUTED),
        )
    } else {
        Text::styled(
            format!("{}  {}", if app.shell_mode { "$" } else { "›" }, app.input),
            Style::default().fg(TEXT),
        )
    };
    frame.render_widget(
        Paragraph::new(content).wrap(Wrap { trim: false }).block(
            Block::default()
                .title(title)
                .borders(Borders::ALL)
                .border_style(Style::default().fg(if app.busy { Color::Yellow } else { ORANGE }))
                .style(Style::default().bg(PANEL)),
        ),
        area,
    );
    if !app.busy && app.palette.is_none() && app.approval.is_none() {
        let before_cursor = format!(
            "{}  {}",
            if app.shell_mode { "$" } else { "›" },
            &app.input[..app.cursor]
        );
        let (column, row) = visual_cursor(&before_cursor, area.width.saturating_sub(2).max(1));
        let x = area
            .x
            .saturating_add(1)
            .saturating_add(column)
            .min(area.right().saturating_sub(2));
        let y = area
            .y
            .saturating_add(1)
            .saturating_add(row)
            .min(area.bottom().saturating_sub(2));
        frame.set_cursor_position((x, y));
    }
}

fn draw_footer(frame: &mut Frame<'_>, area: Rect, app: &App, cfg: &RuntimeConfig) {
    // Hints come from the action registry, so the footer can only ever name a
    // chord that is actually bound, and on a host that cannot deliver
    // `Ctrl+Home` it names the arrow form instead.
    let mut hints: Vec<String> = app
        .keys
        .hints(app.busy)
        .into_iter()
        .map(|(key, label)| format!("{key} {label}"))
        .collect();
    // While the app is not claiming the mouse, the wheel does nothing here —
    // measured, not assumed: Apple Terminal delivers no wheel event at all, and
    // does not implement alternate-scroll either. Say so rather than leaving the
    // developer to conclude that scrolling is broken. The chord still comes from
    // the registry; only the decision to show it lives here, because the
    // registry does not track mouse state.
    if !app.mouse_capture
        && let Some(chord) = app
            .keys
            .advertised_key(crate::tui_keys::ActionId::ToggleMouseCapture)
    {
        hints.push(format!("{} wheel off", chord.display()));
    }
    let mode = if app.shell_mode {
        "SHELL"
    } else {
        app.mode.label()
    };

    // Drop hints from the right until the line fits: the mode and context
    // readings are status the developer is tracking, the trailing hints are
    // reminders, so the reminders yield first.
    let prefix_wide = format!(
        "{mode}  context {}%   planner {}",
        app.context_percent, cfg.planner_model
    );
    let prefix_narrow = format!("{mode}  context {}%", app.context_percent);
    let budget = area.width as usize;
    let mut prefix = if UnicodeWidthStr::width(prefix_wide.as_str()) + 24 <= budget {
        prefix_wide
    } else {
        prefix_narrow
    };
    let mut shown = hints.len();
    loop {
        let candidate = if shown == 0 {
            format!(" {prefix} ")
        } else {
            format!(" {prefix}     {} ", hints[..shown].join("  "))
        };
        if UnicodeWidthStr::width(candidate.as_str()) <= budget || shown == 0 {
            prefix = candidate;
            break;
        }
        shown -= 1;
    }

    frame.render_widget(
        Paragraph::new(prefix).style(Style::default().fg(MUTED)),
        area,
    );
}

fn composer_height(app: &App, width: u16, height: u16) -> u16 {
    let inner = width.saturating_sub(4).max(1) as usize;
    let lines = app
        .input
        .split('\n')
        .map(|line| UnicodeWidthStr::width(line).max(1).div_ceil(inner))
        .sum::<usize>();
    (lines as u16 + 2).clamp(3, 8.min((height / 3).max(3)))
}

fn visual_cursor(text: &str, width: u16) -> (u16, u16) {
    let width = width.max(1);
    let mut column = 0;
    let mut row = 0;
    for character in text.chars() {
        if character == '\n' {
            column = 0;
            row += 1;
            continue;
        }
        let character_width = UnicodeWidthChar::width(character).unwrap_or(0) as u16;
        if column + character_width > width {
            column = 0;
            row += 1;
        }
        column += character_width;
        if column >= width {
            column = 0;
            row += 1;
        }
    }
    (column, row)
}

fn friendly_tool_name(name: &str) -> String {
    match name {
        "execute_bash" => "Run command".into(),
        "fs_read" => "Read file".into(),
        "fs_write" => "Write file".into(),
        "fs_edit" => "Edit file".into(),
        "search_files" | "grep" => "Search code".into(),
        "list_files" => "List files".into(),
        "plan_work" => "Build plan".into(),
        other => other.replace('_', " "),
    }
}

fn friendly_detail(detail: &str, width: usize) -> String {
    let concise = serde_json::from_str::<serde_json::Value>(detail)
        .ok()
        .and_then(|value| value.as_object().cloned())
        .map(|object| {
            object
                .iter()
                .filter_map(|(key, value)| {
                    let value = value
                        .as_str()
                        .map(ToOwned::to_owned)
                        .or_else(|| value.as_i64().map(|value| value.to_string()))
                        .or_else(|| value.as_bool().map(|value| value.to_string()))?;
                    Some(format!("{key}: {value}"))
                })
                .take(2)
                .collect::<Vec<_>>()
                .join("  ·  ")
        })
        .filter(|value| !value.is_empty())
        .unwrap_or_else(|| detail.to_string());
    crate::text::truncate(&concise, width.max(12), "…")
}

/// Render Markdown to styled lines, wrapped to `width`.
///
/// Width-aware on purpose: `width` is the measure to wrap prose at, while code
/// and tables take the full pane. The caller decides the measure — see
/// `App::prose_measure` and the `/width` command.
///
/// Vertical rhythm is deliberately asymmetric. A heading takes a blank line
/// above and none below, so the gap binds the heading to the content it titles
/// instead of floating equidistant between two blocks. Uniform spacing is why
/// the previous output read as undifferentiated.
fn markdown_lines_wrapped(markdown: &str, width: usize) -> Vec<Line<'static>> {
    use crate::tui_text::{WrapStyle, wrap_styled};

    // `width` is the measure, chosen by the caller. Capping here instead left
    // dead space on a wide terminal that reads as a panel that failed to close,
    // so the decision belongs with whoever knows the pane and the preference.
    let pane = width.max(8);
    let measure = pane;

    let mut lines: Vec<Line<'static>> = Vec::new();
    let mut code = false;
    let mut code_block: Vec<String> = Vec::new();

    // Avoid a leading blank line at the very top of a message.
    let space_above = |lines: &mut Vec<Line<'static>>| {
        if !lines.is_empty()
            && lines
                .last()
                .is_some_and(|line| !line.spans.iter().all(|span| span.content.trim().is_empty()))
        {
            lines.push(Line::default());
        }
    };

    for raw in markdown.lines() {
        if let Some(language) = raw.trim().strip_prefix("```") {
            if code {
                // Closing fence: emit the collected block as one surface.
                lines.extend(code_block_lines(&code_block, pane));
                code_block.clear();
                code = false;
            } else {
                code = true;
                space_above(&mut lines);
                lines.push(code_fence_label(language, pane));
            }
            continue;
        }
        if code {
            code_block.push(raw.to_string());
            continue;
        }

        if raw.trim().is_empty() {
            // Collapse runs of blank lines; rhythm comes from the renderer, not
            // from however many newlines the model happened to emit.
            if lines
                .last()
                .is_some_and(|line| line.spans.iter().all(|span| span.content.trim().is_empty()))
            {
                continue;
            }
            lines.push(Line::default());
            continue;
        }

        if let Some(heading) = raw.strip_prefix("### ") {
            space_above(&mut lines);
            lines.extend(wrap_styled(
                &[Span::styled(
                    heading.to_string(),
                    Style::default().fg(TEXT).add_modifier(Modifier::BOLD),
                )],
                &WrapStyle::plain(measure),
            ));
        } else if let Some(heading) = raw.strip_prefix("## ").or_else(|| raw.strip_prefix("# ")) {
            space_above(&mut lines);
            lines.extend(wrap_styled(
                &[Span::styled(
                    heading.to_string(),
                    Style::default().fg(ORANGE).add_modifier(Modifier::BOLD),
                )],
                &WrapStyle::plain(measure),
            ));
        } else if let Some(item) = raw.strip_prefix("- ").or_else(|| raw.strip_prefix("* ")) {
            lines.extend(wrap_styled(
                &inline_spans(item),
                &WrapStyle::hanging(
                    measure,
                    Span::styled("  •  ".to_string(), Style::default().fg(ORANGE)),
                ),
            ));
        } else if let Some(quote) = raw.strip_prefix("> ") {
            let quoted = wrap_styled(
                &inline_spans(quote),
                &WrapStyle::hanging(
                    measure,
                    Span::styled("┃ ".to_string(), Style::default().fg(Color::Cyan)),
                ),
            );
            lines.extend(
                quoted
                    .into_iter()
                    .map(|line| line.style(Style::default().fg(MUTED))),
            );
        } else if raw.trim_start().starts_with('|') {
            // Tables are column-aligned by construction; wrapping would destroy
            // them, so they take the full pane and may scroll horizontally.
            lines.push(Line::from(inline_spans(raw)));
        } else {
            lines.extend(wrap_styled(&inline_spans(raw), &WrapStyle::plain(measure)));
        }
    }

    // An unterminated fence still has to render.
    if !code_block.is_empty() {
        lines.extend(code_block_lines(&code_block, pane));
    }

    if lines.is_empty() {
        lines.push(Line::default());
    }
    lines
}

/// Width of the code surface, bounded so it does not span an entire wide pane.
fn code_surface_width(pane: usize) -> usize {
    pane.saturating_sub(2).clamp(20, 100)
}

/// The language label that opens a code block.
fn code_fence_label(language: &str, pane: usize) -> Line<'static> {
    let label = if language.trim().is_empty() {
        "code"
    } else {
        language.trim()
    };
    let surface = code_surface_width(pane);
    let mut text = format!("  {label}");
    let pad = surface.saturating_sub(text.width());
    text.push_str(&" ".repeat(pad));
    Line::from(Span::styled(
        text,
        Style::default()
            .fg(ORANGE)
            .bg(Color::Rgb(22, 24, 30))
            .add_modifier(Modifier::BOLD),
    ))
}

/// Render a code block as a rectangular surface.
///
/// Each line is padded to a uniform width so the background forms a block. The
/// previous rendering coloured only as far as each line's own text, so the right
/// edge traced the code's line lengths and never read as a surface.
fn code_block_lines(code: &[String], pane: usize) -> Vec<Line<'static>> {
    let background = Color::Rgb(22, 24, 30);
    let surface = code_surface_width(pane);
    let mut lines = Vec::with_capacity(code.len() + 1);

    for raw in code {
        let mut line = code_line(raw);
        let used: usize = line.spans.iter().map(|span| span.content.width()).sum();
        if used < surface {
            line.spans.push(Span::styled(
                " ".repeat(surface - used),
                Style::default().bg(background),
            ));
        }
        lines.push(line);
    }

    // Close the surface with a padded blank row rather than an abrupt edge.
    lines.push(Line::from(Span::styled(
        " ".repeat(surface),
        Style::default().bg(background),
    )));
    lines
}

fn inline_spans(input: &str) -> Vec<Span<'static>> {
    let mut spans = Vec::new();
    let mut rest = input;
    while !rest.is_empty() {
        let bold = rest.find("**");
        let code = rest.find('`');
        let next = match (bold, code) {
            (Some(a), Some(b)) => a.min(b),
            (Some(a), None) => a,
            (None, Some(b)) => b,
            (None, None) => {
                spans.push(Span::styled(rest.to_string(), Style::default().fg(TEXT)));
                break;
            }
        };
        if next > 0 {
            spans.push(Span::styled(
                rest[..next].to_string(),
                Style::default().fg(TEXT),
            ));
            rest = &rest[next..];
            continue;
        }
        if rest.starts_with("**")
            && let Some(end) = rest[2..].find("**")
        {
            spans.push(Span::styled(
                rest[2..2 + end].to_string(),
                Style::default().fg(TEXT).add_modifier(Modifier::BOLD),
            ));
            rest = &rest[2 + end + 2..];
        } else if rest.starts_with('`')
            && let Some(end) = rest[1..].find('`')
        {
            spans.push(Span::styled(
                format!(" {} ", &rest[1..1 + end]),
                Style::default().fg(Color::Cyan).bg(Color::Rgb(35, 38, 46)),
            ));
            rest = &rest[1 + end + 1..];
        } else {
            spans.push(Span::styled(rest.to_string(), Style::default().fg(TEXT)));
            break;
        }
    }
    spans
}

fn code_line(raw: &str) -> Line<'static> {
    let background = Color::Rgb(22, 24, 30);
    let trimmed = raw.trim_start();
    if trimmed.starts_with("//") || trimmed.starts_with('#') {
        return Line::from(Span::styled(
            format!("  {raw}  "),
            Style::default().fg(MUTED).bg(background),
        ));
    }
    let indent_end = raw.len().saturating_sub(trimmed.len());
    let keyword_end = trimmed.find(char::is_whitespace).unwrap_or(trimmed.len());
    let keyword = &trimmed[..keyword_end];
    let keywords = [
        "async", "await", "fn", "let", "pub", "use", "impl", "struct", "enum", "match", "if",
        "else", "for", "while", "return",
    ];
    if keywords.contains(&keyword) {
        Line::from(vec![
            Span::styled(
                format!("  {}", &raw[..indent_end]),
                Style::default().bg(background),
            ),
            Span::styled(
                keyword.to_string(),
                Style::default()
                    .fg(Color::Magenta)
                    .bg(background)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::styled(
                format!("{}  ", &trimmed[keyword_end..]),
                Style::default().fg(CODE_TEXT).bg(background),
            ),
        ])
    } else {
        Line::from(Span::styled(
            format!("  {raw}  "),
            Style::default().fg(CODE_TEXT).bg(background),
        ))
    }
}

fn centered(width: u16, height: u16, area: Rect) -> Rect {
    let vertical = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Percentage((100 - height) / 2),
            Constraint::Percentage(height),
            Constraint::Percentage((100 - height) / 2),
        ])
        .split(area);
    Layout::default()
        .direction(Direction::Horizontal)
        .constraints([
            Constraint::Percentage((100 - width) / 2),
            Constraint::Percentage(width),
            Constraint::Percentage((100 - width) / 2),
        ])
        .split(vertical[1])[1]
}

fn format_mcp_markdown(cfg: &RuntimeConfig, runtime_tools: &ResolvedRuntimeTools) -> String {
    let mut output = format!("## MCP servers — profile `{}`\n\n", cfg.profile);
    if cfg.mcp_servers.is_empty() {
        output.push_str("No MCP servers are configured.\n");
        return output;
    }
    for server in &cfg.mcp_servers {
        output.push_str(&format!(
            "- **{}** — {} · {} · `{}`\n",
            server.name,
            if server.enabled.unwrap_or(true) {
                "enabled"
            } else {
                "disabled"
            },
            if server.is_stdio() {
                "stdio"
            } else {
                "streamable HTTP"
            },
            server.display_target(),
        ));
    }
    output.push_str(&format!(
        "\n**Connected MCP tools:** {}\n\n",
        runtime_tools.mcp_tool_names().len()
    ));
    for tool in runtime_tools.mcp_tool_names() {
        output.push_str(&format!("- `{tool}`\n"));
    }
    let failures = runtime_tools.connect_failure_report();
    if !failures.is_empty() {
        // Requirement 10.4: an unreachable server must be visible here, not only
        // in a log sink the alternate screen hides.
        output.push_str(&format!(
            "\n**Unreachable servers:** {}\n\n",
            failures.len()
        ));
        for failure in &failures {
            output.push_str(&format!("- {failure}\n"));
        }
    }
    output.push_str("\nUse `/doctor` for configuration readiness and `zavora-cli mcp doctor` for live protocol diagnostics.");
    output
}

fn format_mcp_doctor_markdown(cfg: &RuntimeConfig, runtime_tools: &ResolvedRuntimeTools) -> String {
    let mut output = String::from("## MCP configuration check\n\n");
    if cfg.mcp_servers.is_empty() {
        output.push_str("No MCP servers are configured.");
        return output;
    }
    for server in &cfg.mcp_servers {
        let status = if !server.enabled.unwrap_or(true) {
            "disabled".to_string()
        } else if let Some(hint) = crate::mcp::check_auth_hint(server) {
            format!("authentication needs attention: {hint}")
        } else {
            "configuration ready".to_string()
        };
        output.push_str(&format!("- **{}** — {}\n", server.name, status));
    }
    output.push_str(&format!(
        "\n- **Runtime discovery:** {} MCP tool(s) connected\n",
        runtime_tools.mcp_tool_names().len()
    ));
    output.push_str(
        "\nRun `zavora-cli mcp doctor [--server NAME]` for network and protocol diagnostics.",
    );
    output
}

fn format_agents_markdown() -> String {
    let mut output = String::from("## Agents\n\n");
    match crate::capabilities::CapabilitySnapshot::load(&[], &[]) {
        Ok(snapshot) if !snapshot.agents.is_empty() => {
            for agent in snapshot.agents {
                output.push_str(&format!(
                    "- **{}** — {} · {}\n",
                    agent.name,
                    agent.source,
                    if agent.description.is_empty() {
                        "No description"
                    } else {
                        &agent.description
                    }
                ));
            }
        }
        Ok(_) => output.push_str("No agents are configured.\n"),
        Err(error) => output.push_str(&format!("Agent catalog could not be loaded: {error}\n")),
    }
    output.push_str("\nThe coordinator delegates focused work automatically. Use `zavora-cli agents run --name NAME TASK` for direct execution.");
    output
}

fn draw_palette(frame: &mut Frame<'_>, area: Rect, app: &App) {
    let popup = centered(72, 72, area);
    frame.render_widget(Clear, popup);
    let Some(palette) = app.palette.as_ref() else {
        return;
    };
    let matches = matching_commands(&palette.query);
    let visible = popup.height.saturating_sub(6) as usize;
    let selected = palette.selected.min(matches.len().saturating_sub(1));
    let start = selected.saturating_sub(visible.saturating_sub(1));
    let mut lines = vec![
        Line::from(vec![
            Span::styled("Search  ", Style::default().fg(MUTED)),
            Span::styled(
                if palette.query.is_empty() {
                    "type a command or capability…".to_string()
                } else {
                    palette.query.clone()
                },
                Style::default().fg(TEXT).add_modifier(Modifier::BOLD),
            ),
        ]),
        Line::from(Span::styled(
            "─".repeat(popup.width.saturating_sub(4) as usize),
            Style::default().fg(Color::DarkGray),
        )),
    ];
    if matches.is_empty() {
        lines.push(Line::from(Span::styled(
            "No matching commands",
            Style::default().fg(MUTED),
        )));
    } else {
        for (index, spec) in matches.iter().enumerate().skip(start).take(visible) {
            let active = index == selected;
            lines.push(Line::from(vec![
                Span::styled(
                    if active { "› " } else { "  " },
                    Style::default().fg(ORANGE),
                ),
                Span::styled(
                    format!("{:<20}", spec.usage),
                    Style::default()
                        .fg(if active { Color::Black } else { Color::Cyan })
                        .bg(if active { ORANGE } else { PANEL })
                        .add_modifier(Modifier::BOLD),
                ),
                Span::styled(
                    format!("  {}", spec.description),
                    Style::default()
                        .fg(if active { Color::Black } else { TEXT })
                        .bg(if active { ORANGE } else { PANEL }),
                ),
            ]));
        }
    }
    lines.push(Line::from(Span::styled(
        "↑↓ navigate  ·  Enter choose  ·  Esc close",
        Style::default().fg(MUTED),
    )));
    frame.render_widget(
        Paragraph::new(lines)
            .block(
                Block::default()
                    .title(" Keyboard & actions ")
                    .borders(Borders::ALL)
                    .border_style(Style::default().fg(ORANGE)),
            )
            .style(Style::default().bg(PANEL).fg(TEXT)),
        popup,
    );
}

fn draw_command_suggestions(frame: &mut Frame<'_>, composer: Rect, app: &App) {
    let suggestions = slash_suggestions(&app.input);
    if suggestions.is_empty() || app.input == "/exit" {
        return;
    }
    let height = (suggestions.len() as u16 + 2).min(9);
    let width = composer.width.min(78);
    let y = composer.y.saturating_sub(height);
    let popup = Rect::new(composer.x, y, width, height);
    frame.render_widget(Clear, popup);
    let items = suggestions.into_iter().map(|spec| {
        ListItem::new(Line::from(vec![
            Span::styled(
                format!("{:<20}", spec.usage),
                Style::default()
                    .fg(Color::Cyan)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::styled(spec.description, Style::default().fg(MUTED)),
        ]))
    });
    frame.render_widget(
        List::new(items).block(
            Block::default()
                .title(" Commands  ·  Tab complete ")
                .borders(Borders::ALL)
                .border_style(Style::default().fg(ORANGE)),
        ),
        popup,
    );
}

fn draw_approval(frame: &mut Frame<'_>, area: Rect, approval: &PendingApproval) {
    let popup = centered(70, 46, area);
    frame.render_widget(Clear, popup);
    let text = Text::from(vec![
        Line::from(Span::styled(
            "Permission required",
            Style::default()
                .fg(Color::Yellow)
                .add_modifier(Modifier::BOLD),
        )),
        Line::from(""),
        Line::from(vec![
            Span::styled("Tool  ", Style::default().fg(MUTED)),
            Span::styled(
                &approval.tool,
                Style::default().fg(TEXT).add_modifier(Modifier::BOLD),
            ),
        ]),
        Line::from(""),
        Line::from(crate::text::truncate(&approval.detail, 500, "…")),
        Line::from(""),
        if approval.enables_agent_mode {
            Line::from(vec![
                Span::styled(
                    "Y",
                    Style::default()
                        .fg(Color::Green)
                        .add_modifier(Modifier::BOLD),
                ),
                Span::raw(" enable for session   "),
                Span::styled(
                    "N",
                    Style::default().fg(Color::Red).add_modifier(Modifier::BOLD),
                ),
                Span::raw(" cancel"),
            ])
        } else {
            Line::from(vec![
                Span::styled(
                    "Y",
                    Style::default()
                        .fg(Color::Green)
                        .add_modifier(Modifier::BOLD),
                ),
                Span::raw(" allow once   "),
                Span::styled(
                    "T",
                    Style::default()
                        .fg(Color::Cyan)
                        .add_modifier(Modifier::BOLD),
                ),
                Span::raw(" trust session   "),
                Span::styled(
                    "N",
                    Style::default().fg(Color::Red).add_modifier(Modifier::BOLD),
                ),
                Span::raw(" deny"),
            ])
        },
    ]);
    frame.render_widget(
        Paragraph::new(text)
            .wrap(Wrap { trim: false })
            .block(
                Block::default()
                    .title(" Approval ")
                    .borders(Borders::ALL)
                    .border_style(Style::default().fg(Color::Yellow)),
            )
            .style(Style::default().bg(PANEL).fg(TEXT)),
        popup,
    );
}

#[cfg(test)]
mod tests {

    /// `/copy` defaults to the last response, which is what a developer wants
    /// after asking for code — not the whole transcript.
    #[test]
    fn copy_defaults_to_the_last_response() {
        let mut app = App::new();
        app.push_message(Message::new("YOU", "write the function"));
        app.push_message(Message::new("ZAVORA", "fn answer() -> u8 { 42 }"));
        app.push_message(Message::new("YOU", "thanks"));

        let payload = clipboard_payload(&app, false).expect("a response exists");
        assert_eq!(payload, "fn answer() -> u8 { 42 }");
        assert!(
            !payload.contains("write the function"),
            "the prompt leaked into the copied response"
        );
    }

    #[test]
    fn copy_all_includes_both_sides_and_skips_the_elision_marker() {
        let mut app = App::new();
        app.push_message(Message::new("YOU", "question"));
        app.push_message(Message::new("ZAVORA", "answer"));
        app.push_message(Message::new(ELIDED_ROLE, "12 earlier messages elided"));

        let payload = clipboard_payload(&app, true).expect("transcript exists");
        assert!(payload.contains("question"));
        assert!(payload.contains("answer"));
        assert!(
            !payload.contains("earlier messages elided"),
            "the elision marker was copied as content"
        );
    }

    #[test]
    fn copy_reports_nothing_to_copy_on_an_empty_transcript() {
        let app = App::new();
        assert!(clipboard_payload(&app, false).is_none());
        assert!(clipboard_payload(&app, true).is_none());
    }

    /// The workspace claims the wheel at startup, and can hand it back.
    ///
    /// Not the obvious default, since claiming the mouse costs the terminal's own
    /// click-drag selection. It is the correct one because the alternative is
    /// destructive rather than merely inert: with the wheel left to the terminal,
    /// scrolling in the alternate screen moves the terminal's own buffer and
    /// displaces the frame the app is drawing into, after which diffed redraws
    /// land at the wrong row and the transcript interleaves old and new text.
    /// Selection stays one modifier away, and `Ctrl+R` gives the mouse back.
    #[test]
    fn the_wheel_is_claimed_by_default_and_can_be_handed_back() {
        let mut app = App::new();
        assert!(
            app.mouse_capture,
            "the wheel must scroll the transcript rather than the terminal"
        );
        app.toggle_mouse_capture();
        assert!(
            !app.mouse_capture,
            "Ctrl+R must hand the mouse back for native selection"
        );
        app.toggle_mouse_capture();
        assert!(app.mouse_capture, "and take it again");
    }

    /// Property 13: retained buffers stay bounded, and elision is visible.
    #[test]
    fn the_transcript_buffer_is_bounded_with_a_visible_marker() {
        let mut app = App::new();
        for n in 0..(MAX_RETAINED_MESSAGES + 25) {
            app.push_message(Message::new("YOU", format!("message {n}")));
        }

        assert_eq!(
            app.messages.len(),
            MAX_RETAINED_MESSAGES,
            "transcript exceeded its cap"
        );

        let markers = app
            .messages
            .iter()
            .filter(|message| message.role == ELIDED_ROLE)
            .count();
        assert_eq!(markers, 1, "expected exactly one elision marker");
        assert_eq!(app.messages[0].role, ELIDED_ROLE);
        assert!(
            app.messages[0].text.contains("earlier messages elided"),
            "marker text is not self-describing: {:?}",
            app.messages[0].text
        );

        // The most recent message must survive.
        assert!(
            app.messages.last().is_some_and(|last| last
                .text
                .ends_with(&format!("{}", MAX_RETAINED_MESSAGES + 24))),
            "the newest message was dropped"
        );
    }

    /// Property 13: the marker accumulates rather than resetting, so the count
    /// stays truthful across repeated elisions.
    #[test]
    fn the_elision_marker_accumulates() {
        let mut app = App::new();
        for n in 0..(MAX_RETAINED_MESSAGES + 10) {
            app.push_message(Message::new("YOU", format!("a{n}")));
        }
        let first_count = app.messages[0].text.clone();
        for n in 0..20 {
            app.push_message(Message::new("YOU", format!("b{n}")));
        }
        let second_count = app.messages[0].text.clone();
        assert_ne!(
            first_count, second_count,
            "the elision count did not grow after further elision"
        );
    }

    /// Property 13: prompt history is bounded too.
    #[test]
    fn prompt_history_is_bounded() {
        let mut app = App::new();
        for n in 0..(MAX_PROMPT_HISTORY + 50) {
            app.remember_input(&format!("prompt {n}"));
        }
        assert_eq!(app.history.len(), MAX_PROMPT_HISTORY);
        assert_eq!(
            app.history.last().map(String::as_str),
            Some(format!("prompt {}", MAX_PROMPT_HISTORY + 49).as_str())
        );
    }

    /// Requirement 4.6: a message renders once per revision, not once per frame.
    #[test]
    fn message_rendering_is_cached_until_the_text_changes() {
        let mut message = Message::new("ZAVORA", "## Heading\n\nBody text.");

        let first = message.lines(60);
        assert!(!first.is_empty());
        // Cache populated at the current text length and width.
        assert_eq!(
            message
                .rendered
                .borrow()
                .as_ref()
                .map(|(len, width, _)| (*len, *width)),
            Some((message.text.len(), 60))
        );

        // A second read at the same width must reuse the cache.
        let second = message.lines(60);
        assert_eq!(first.len(), second.len());

        // Appending invalidates it.
        message.append(" More text.");
        assert!(
            message.rendered.borrow().is_none(),
            "appending did not invalidate the render cache"
        );
        let third = message.lines(60);
        assert!(third.len() >= first.len());

        // A different width must invalidate too, since wrapping depends on it.
        let narrow = message.lines(24);
        assert!(
            narrow.len() >= third.len(),
            "a narrower measure should produce at least as many lines"
        );
    }
    use super::*;
    use ratatui::Terminal;
    use ratatui::backend::TestBackend;

    #[test]
    fn unicode_cursor_tracks_terminal_width() {
        assert_eq!(visual_cursor("›  hello", 40), (8, 0));
        assert_eq!(visual_cursor("›  crab 🦀", 40), (10, 0));
        assert_eq!(visual_cursor("12345", 4), (1, 1));
        assert_eq!(visual_cursor("one\ntwo", 40), (3, 1));
    }

    #[test]
    fn word_navigation_uses_utf8_safe_boundaries() {
        let text = "hello brave 🦀 world";
        let world = text.find("world").unwrap();
        assert_eq!(previous_word_boundary(text, text.len()), world);
        assert_eq!(next_word_boundary(text, 0), 5);
        assert_eq!(previous_word_boundary("🦀 tools", "🦀 tools".len()), 5);
    }

    #[test]
    fn parallel_same_name_tools_are_correlated_by_call_id() {
        let mut app = App::new();
        for call_id in ["call-a", "call-b"] {
            app.apply(UiEvent::ToolStarted {
                call_id: Some(call_id.into()),
                name: "fs_read".into(),
                detail: format!(r#"{{"path":"{call_id}.rs"}}"#),
            });
        }
        app.apply(UiEvent::ToolFinished {
            call_id: Some("call-b".into()),
            name: "fs_read".into(),
            success: true,
            detail: "done".into(),
        });

        assert!(matches!(app.activities[0].state, ActivityState::Running));
        assert!(matches!(app.activities[1].state, ActivityState::Passed));
    }

    #[test]
    fn start_state_and_composer_render_at_common_terminal_sizes() {
        for (width, height) in [(120, 40), (80, 24), (60, 18)] {
            let backend = TestBackend::new(width, height);
            let mut terminal = Terminal::new(backend).unwrap();
            let app = App::new();
            terminal
                .draw(|frame| {
                    let area = frame.area();
                    let parts = Layout::default()
                        .direction(Direction::Vertical)
                        .constraints([Constraint::Min(5), Constraint::Length(3)])
                        .split(area);
                    draw_welcome(frame, parts[0]);
                    draw_composer(frame, parts[1], &app);
                })
                .unwrap();
            let rendered = terminal
                .backend()
                .buffer()
                .content()
                .iter()
                .map(|cell| cell.symbol())
                .collect::<String>();
            assert!(rendered.contains("ZAVORA"));
            assert!(rendered.contains("Ask Zavora"));
        }
    }

    #[test]
    fn markdown_renderer_styles_code_without_losing_content() {
        let lines = markdown_lines_wrapped(
            "## Example\nUse `cargo check`.\n```rust\nlet ready = true;\n```",
            60,
        );
        let rendered = lines
            .iter()
            .map(ToString::to_string)
            .collect::<Vec<_>>()
            .join("\n");
        assert!(rendered.contains("Example"));
        assert!(rendered.contains("cargo check"));
        assert!(rendered.contains("let ready = true;"));
    }

    #[test]
    fn command_registry_is_unique_and_searchable() {
        let names = COMMAND_SPECS
            .iter()
            .map(|spec| spec.name)
            .collect::<std::collections::BTreeSet<_>>();
        assert_eq!(names.len(), COMMAND_SPECS.len());

        let matches = matching_commands("conversation state");
        assert!(matches.iter().any(|spec| spec.name == "/checkpoint"));
        assert!(matches.iter().all(|spec| spec.category == "Session"));
    }

    #[test]
    fn slash_completion_prefers_exact_prefixes_and_preserves_argument_space() {
        let suggestions = slash_suggestions("/pla");
        assert_eq!(suggestions[0].name, "/planner-provider");
        assert!(command_input(suggestions[0]).ends_with(' '));

        let exact = slash_suggestions("/status");
        assert_eq!(exact.len(), 1);
        assert_eq!(command_input(exact[0]), "/status");
    }

    #[test]
    fn prompt_history_restores_the_unsubmitted_draft() {
        let mut app = App::new();
        app.remember_input("first");
        app.remember_input("second");
        app.replace_input("draft");

        app.history_previous();
        assert_eq!(app.input, "second");
        app.history_previous();
        assert_eq!(app.input, "first");
        app.history_next();
        assert_eq!(app.input, "second");
        app.history_next();
        assert_eq!(app.input, "draft");
    }

    #[test]
    fn system_events_and_mouse_mode_switch_update_app_state() {
        let mut app = App::new();
        app.apply(UiEvent::System("ready".into()));
        assert_eq!(
            app.messages.last().map(|message| message.text.as_str()),
            Some("ready")
        );

        handle_mouse_click(&mut app, 1, 0, Rect::new(0, 0, 120, 40));
        assert_eq!(app.mode, Mode::Plan);
    }

    #[test]
    fn searchable_palette_renders_selected_commands() {
        let backend = TestBackend::new(100, 32);
        let mut terminal = Terminal::new(backend).unwrap();
        let mut app = App::new();
        app.palette = Some(PaletteState {
            query: "session".into(),
            selected: 0,
        });
        terminal
            .draw(|frame| draw_palette(frame, frame.area(), &app))
            .unwrap();
        let rendered = terminal
            .backend()
            .buffer()
            .content()
            .iter()
            .map(|cell| cell.symbol())
            .collect::<String>();
        assert!(rendered.contains("/sessions"));
        assert!(rendered.contains("/checkpoint"));
    }

    #[test]
    fn transcript_export_formats_markdown() {
        let mut app = App::new();
        app.push_message(Message::new("YOU", "ship it"));
        let markdown =
            format_transcript_markdown(&app, "test-session", "default", "openai", "gpt-test");
        assert!(markdown.contains("# Zavora session"));
        assert!(markdown.contains("## YOU\n\nship it"));
    }
}

#[cfg(test)]
mod typography_tests {
    use super::*;
    use ratatui::Terminal;
    use ratatui::backend::TestBackend;

    const SAMPLE: &str = "## Measure matters\n\nLong lines make the eye lose its place on the return sweep, which reads as a wall of text even when the spacing is otherwise correct. Capping the measure is the single largest change.\n\n- A list item long enough that it must wrap, so the hanging indent has something to prove\n- A short one\n\n```rust\nfn answer() -> u8 { 42 }\n```\n\n> A quoted line that also runs long enough to wrap onto a second line of output.\n";

    fn render(width: u16, height: u16) -> String {
        let mut app = App::new();
        app.push_message(Message::new("YOU", "why does this look better"));
        app.push_message(Message::new("ZAVORA", SAMPLE));

        let backend = TestBackend::new(width, height);
        let mut terminal = Terminal::new(backend).expect("terminal");
        terminal
            .draw(|frame| {
                draw_transcript(frame, frame.area(), &app);
            })
            .expect("draw");

        let buffer = terminal.backend().buffer().clone();
        // Drop the right border column and the bottom border row: they are the
        // block's chrome, not content, and would skew every measurement.
        (0..buffer.area.height.saturating_sub(1))
            .map(|row| {
                (0..buffer.area.width.saturating_sub(1))
                    .map(|column| buffer[(column, row)].symbol())
                    .collect::<String>()
                    .trim_end()
                    .to_string()
            })
            .collect::<Vec<_>>()
            .join("\n")
    }

    /// Every content row starts with the gutter, never flush at column zero.
    #[test]
    fn content_is_inset_by_the_gutter() {
        let output = render(100, 40);
        let content_rows: Vec<&str> = output
            .lines()
            // The block title occupies the first row and is chrome.
            .skip(1)
            .filter(|line| !line.trim().is_empty())
            .collect();
        assert!(!content_rows.is_empty());
        for row in content_rows {
            assert!(
                row.starts_with(' '),
                "row was flush against the edge: {row:?}"
            );
        }
    }

    /// A wrapped bullet keeps its alignment.
    #[test]
    fn wrapped_bullets_stay_aligned() {
        let output = render(60, 40);
        let bullet_row = output
            .lines()
            .position(|line| line.contains('•'))
            .expect("a bullet was rendered");
        let rows: Vec<&str> = output.lines().collect();
        let bullet_column = rows[bullet_row].find('•').expect("bullet column");
        let continuation = rows[bullet_row + 1];
        // The continuation must be indented at least to the bullet's text.
        let first_text = continuation
            .find(|c: char| !c.is_whitespace())
            .unwrap_or(usize::MAX);
        assert!(
            first_text > bullet_column,
            "continuation at column {first_text} is not hanging under the bullet at {bullet_column}: {continuation:?}"
        );
    }

    /// No horizontal rule between messages; space does the separating.
    #[test]
    fn messages_are_separated_by_space_not_rules() {
        let output = render(100, 40);
        assert!(
            !output.contains("────────"),
            "a horizontal rule survived:\n{output}"
        );
    }

    /// The default hides the pane while a turn runs and reveals it afterwards.
    #[test]
    fn autohide_is_the_default_and_tracks_the_turn() {
        let mut app = App::new();
        assert_eq!(app.activity_visibility, ActivityVisibility::AutoHide);

        // Nothing has run: nothing to show.
        assert!(!app.show_activity());

        // A turn starts and records activity — hidden, so the response has room.
        app.busy = true;
        app.activities.push(Activity {
            call_id: None,
            name: "fs_read".into(),
            detail: "src/main.rs".into(),
            state: ActivityState::Running,
            started: Instant::now(),
            elapsed: None,
        });
        assert!(!app.show_activity(), "autohide should hide while busy");

        // The turn ends — the record becomes visible.
        app.busy = false;
        assert!(app.show_activity(), "autohide should reveal after the run");
    }

    #[test]
    fn show_pins_the_pane_and_off_never_shows_it() {
        let mut app = App::new();

        app.activity_visibility = ActivityVisibility::Show;
        assert!(app.show_activity(), "show should pin even when empty");
        app.busy = true;
        assert!(app.show_activity(), "show should stay pinned while busy");

        app.activity_visibility = ActivityVisibility::Off;
        assert!(!app.show_activity());
        app.busy = false;
        app.activities.push(Activity {
            call_id: None,
            name: "grep".into(),
            detail: "TODO".into(),
            state: ActivityState::Passed,
            started: Instant::now(),
            elapsed: None,
        });
        assert!(!app.show_activity(), "off should never show the pane");
    }

    #[test]
    fn activity_visibility_parses_and_cycles() {
        assert_eq!(
            ActivityVisibility::parse("show"),
            Some(ActivityVisibility::Show)
        );
        assert_eq!(
            ActivityVisibility::parse("AUTO"),
            Some(ActivityVisibility::AutoHide)
        );
        assert_eq!(
            ActivityVisibility::parse("never"),
            Some(ActivityVisibility::Off)
        );
        assert_eq!(ActivityVisibility::parse("sideways"), None);

        // Cycling from the default returns to it after three steps.
        let start = ActivityVisibility::AutoHide;
        assert_eq!(start.next().next().next(), start);
    }

    /// The point of the change: when hidden, the pane's chrome is absent and the
    /// transcript owns the full width.
    #[test]
    fn hiding_the_pane_removes_its_chrome_from_the_frame() {
        fn frame_text(visibility: ActivityVisibility, busy: bool) -> String {
            let mut app = App::new();
            app.activity_visibility = visibility;
            app.busy = busy;
            app.activities.push(Activity {
                call_id: None,
                name: "fs_read".into(),
                detail: "src/main.rs".into(),
                state: ActivityState::Passed,
                started: Instant::now(),
                elapsed: None,
            });

            let backend = TestBackend::new(200, 24);
            let mut terminal = Terminal::new(backend).expect("terminal");
            let cfg = crate::tests::base_cfg();
            terminal
                .draw(|frame| draw(frame, &app, &cfg))
                .expect("draw");
            let buffer = terminal.backend().buffer().clone();
            (0..buffer.area.height)
                .map(|row| {
                    (0..buffer.area.width)
                        .map(|column| buffer[(column, row)].symbol())
                        .collect::<String>()
                })
                .collect::<Vec<_>>()
                .join("\n")
        }

        // Pinned: the pane and its contents are on screen.
        let pinned = frame_text(ActivityVisibility::Show, false);
        assert!(
            pinned.contains("Run history"),
            "pinned pane was not rendered"
        );
        assert!(
            pinned.contains("Read file"),
            "pinned pane lost its contents:\n{pinned}"
        );

        // Off: no title, no contents, at any width.
        let off = frame_text(ActivityVisibility::Off, false);
        assert!(
            !off.contains("Run history") && !off.contains("Live run"),
            "the pane's chrome survived being turned off"
        );
        assert!(
            !off.contains("Read file"),
            "the pane's contents survived being turned off"
        );

        // AutoHide while busy: hidden, so the response has the room.
        let busy = frame_text(ActivityVisibility::AutoHide, true);
        assert!(
            !busy.contains("Live run") && !busy.contains("Read file"),
            "autohide showed the pane during a run"
        );

        // AutoHide once idle: revealed, so the record is available.
        let idle = frame_text(ActivityVisibility::AutoHide, false);
        assert!(
            idle.contains("Run history") && idle.contains("Read file"),
            "autohide did not reveal the pane after the run"
        );
    }

    /// A long response must be scrollable all the way back to its first line.
    #[test]
    fn a_long_story_can_be_scrolled_to_its_beginning() {
        // A story far taller than any viewport.
        let story = (1..=400)
            .map(|n| format!("Paragraph {n} of the story."))
            .collect::<Vec<_>>()
            .join("\n\n");

        let mut app = App::new();
        app.push_message(Message::new("YOU", "tell me a long story"));
        app.push_message(Message::new("ZAVORA", story));

        let mut terminal = Terminal::new(TestBackend::new(100, 24)).expect("terminal");
        let cfg = crate::tests::base_cfg();
        let draw_once = |app: &App, terminal: &mut Terminal<TestBackend>| -> String {
            terminal.draw(|frame| draw(frame, app, &cfg)).expect("draw");
            let buffer = terminal.backend().buffer().clone();
            (0..buffer.area.height)
                .map(|row| {
                    (0..buffer.area.width)
                        .map(|column| buffer[(column, row)].symbol())
                        .collect::<String>()
                })
                .collect::<Vec<_>>()
                .join("\n")
        };

        // First draw publishes the bounds the key handler clamps against.
        let bottom = draw_once(&app, &mut terminal);
        assert!(
            bottom.contains("Paragraph 400"),
            "should start pinned to the newest output"
        );
        let max = app.max_scroll.get();
        assert!(max > 0, "a long story must be scrollable, max_scroll={max}");

        // Jump to the start: the very first line of the conversation is visible.
        app.scroll_to_start();
        let top = draw_once(&app, &mut terminal);
        assert!(
            top.contains("tell me a long story"),
            "could not reach the beginning of the conversation:\n{}",
            top.lines().take(4).collect::<Vec<_>>().join("\n")
        );
        assert!(
            top.contains("Paragraph 1 of the story."),
            "the story's first paragraph was not reachable"
        );

        // And back to the newest output.
        app.scroll_to_end();
        let bottom_again = draw_once(&app, &mut terminal);
        assert!(bottom_again.contains("Paragraph 400"));
    }

    /// Paging up must not accumulate offset beyond the top.
    ///
    /// Regression: the offset was only clamped at render, so pressing PageUp past
    /// the start grew a counter the view could not reflect, and PageDown then had
    /// to walk back through that dead range before anything moved.
    #[test]
    fn paging_past_the_top_does_not_accumulate_dead_offset() {
        let mut app = App::new();
        app.push_message(Message::new("ZAVORA", "line\n\n".repeat(60)));
        app.max_scroll.set(50);
        app.viewport.set(23);

        for _ in 0..40 {
            let step = app.page_step();
            app.scroll_up(step);
        }
        assert_eq!(
            app.top_offset(),
            0,
            "scroll ran past the top instead of clamping"
        );

        // One page down must move immediately, not absorb dead range.
        let step = app.page_step();
        app.scroll_down(step);
        assert_eq!(app.top_offset(), step, "PageDown did not respond at once");
    }

    /// A page step is a viewport, not a fixed guess.
    #[test]
    fn a_page_step_follows_the_viewport_height() {
        let app = App::new();
        app.viewport.set(40);
        assert_eq!(app.page_step(), 39);
        app.viewport.set(10);
        assert_eq!(app.page_step(), 9);
        // Degenerate heights still make progress.
        app.viewport.set(0);
        assert_eq!(app.page_step(), 1);
    }

    /// The footer shows registry hints and sheds them rather than overflowing.
    #[test]
    fn the_footer_is_generated_and_degrades_on_narrow_terminals() {
        fn footer(width: u16, busy: bool) -> String {
            let mut app = App::new();
            app.busy = busy;
            app.context_percent = 42;
            let cfg = crate::tests::base_cfg();
            let mut terminal = Terminal::new(TestBackend::new(width, 1)).expect("terminal");
            terminal
                .draw(|frame| draw_footer(frame, frame.area(), &app, &cfg))
                .expect("draw");
            let buffer = terminal.backend().buffer().clone();
            (0..buffer.area.width)
                .map(|col| buffer[(col, 0)].symbol())
                .collect::<String>()
        }

        // Wide: status, then hints in priority order.
        let wide = footer(160, false);
        assert!(wide.contains("context 42%"), "status is missing: {wide:?}");
        for expected in ["mode", "send", "history", "scroll", "actions"] {
            assert!(
                wide.contains(expected),
                "the footer dropped {expected:?}: {wide:?}"
            );
        }
        // Priority order, left to right.
        let mode = wide.find("mode").expect("mode");
        let history = wide.find("history").expect("history");
        let scroll = wide.find("scroll").expect("scroll");
        assert!(
            mode < history && history < scroll,
            "hints out of order: {wide:?}"
        );

        // Narrow: hints are shed from the right; the status reading survives and
        // nothing wraps or is truncated mid-cell.
        let narrow = footer(46, false);
        assert!(
            narrow.contains("context 42%"),
            "status must survive a narrow terminal: {narrow:?}"
        );
        assert!(
            narrow.trim_end().chars().count() <= 46,
            "the footer overflowed its line: {narrow:?}"
        );
        assert!(
            !narrow.contains("actions"),
            "the lowest-priority hint should yield first: {narrow:?}"
        );

        // While a turn runs, cancel leads and prompt history is not offered.
        let busy = footer(160, true);
        assert!(busy.contains("cancel"), "no way to cancel shown: {busy:?}");
        assert!(
            !busy.contains("history"),
            "history recall is not available mid-turn: {busy:?}"
        );
    }

    /// The wheel state is visible, and the chord to change it comes from the
    /// registry.
    ///
    /// A dead wheel with nothing on screen to explain it is what makes a
    /// workspace look broken, so the footer says so while the app is not
    /// claiming the mouse.
    #[test]
    fn the_footer_says_when_the_wheel_is_doing_nothing() {
        fn footer(mouse_capture: bool) -> String {
            let mut app = App::new();
            app.mouse_capture = mouse_capture;
            let cfg = crate::tests::base_cfg();
            let mut terminal = Terminal::new(TestBackend::new(170, 1)).expect("terminal");
            terminal
                .draw(|frame| draw_footer(frame, frame.area(), &app, &cfg))
                .expect("draw");
            let buffer = terminal.backend().buffer().clone();
            (0..buffer.area.width)
                .map(|col| buffer[(col, 0)].symbol())
                .collect::<String>()
        }

        let off = footer(false);
        assert!(
            off.contains("wheel off"),
            "a dead wheel must be visible: {off:?}"
        );
        let chord = crate::tui_keys::ActionRegistry::detect()
            .advertised_key(crate::tui_keys::ActionId::ToggleMouseCapture)
            .expect("the toggle must be reachable")
            .display();
        assert!(
            off.contains(&chord),
            "the footer must name the chord that revives it ({chord}): {off:?}"
        );

        let on = footer(true);
        assert!(
            !on.contains("wheel off"),
            "no warning once the wheel works: {on:?}"
        );
    }

    /// Toggling names the cost in both directions, and the modifier that works.
    #[test]
    fn toggling_the_wheel_explains_what_it_costs() {
        let mut app = App::new();
        let modifier = app.keys.terminal().native_selection_modifier();

        // Starts claimed, so the first toggle hands the mouse back.
        app.toggle_mouse_capture();
        assert!(!app.mouse_capture);
        let off = &app.messages.last().expect("a message").text;
        assert!(off.contains("**off**"), "state not stated: {off}");
        assert!(
            off.contains("push this frame out of place"),
            "handing the wheel back has a real cost that must be named: {off}"
        );
        assert!(
            off.contains("PageUp"),
            "and must say what to scroll with instead: {off}"
        );

        app.toggle_mouse_capture();
        assert!(app.mouse_capture);
        let on = &app.messages.last().expect("a message").text;
        assert!(on.contains("**on**"), "state not stated: {on}");
        assert!(
            on.contains(modifier),
            "claiming the wheel costs native selection, so the message must name \
             the modifier that restores it ({modifier}): {on}"
        );

        // Either direction repaints, because a toggle usually follows the
        // terminal having displaced the frame.
        assert!(app.force_redraw, "a toggle must force a full repaint");
    }

    /// `Ctrl+L` repaints first and only clears on a prompt second press.
    ///
    /// Regression: it cleared the conversation on the first press, so the
    /// universal reflex for a corrupted screen destroyed the transcript instead
    /// of repairing it.
    #[test]
    fn redraw_precedes_clearing_the_conversation() {
        let mut app = App::new();
        app.push_message(Message::new("YOU", "keep me"));

        assert!(
            !app.request_redraw(),
            "one press must not clear the conversation"
        );
        assert!(app.force_redraw, "but it must repaint");
        assert_eq!(app.messages.len(), 1, "the transcript survived");

        // A prompt second press means clear.
        assert!(
            app.request_redraw(),
            "a second press within the window should clear"
        );

        // After acting, the window closes: the next press repaints again.
        assert!(
            !app.request_redraw(),
            "the double-press window must not stay armed"
        );
    }

    /// The wheel step is configurable within bounds and rejects nonsense.
    #[test]
    fn the_wheel_step_is_configurable() {
        let app = App::new();
        assert_eq!(app.wheel_lines, 3, "three matches vim and Claude Code");

        // Exercised through the scroll path so the setting is actually used.
        let mut app = App::new();
        app.max_scroll.set(100);
        app.wheel_lines = 7;
        app.scroll_up(app.wheel_lines);
        assert_eq!(
            app.top_offset(),
            93,
            "one notch should move seven lines back from the tail"
        );
    }

    /// A scrolled-back view must not drift while output streams in.
    ///
    /// Regression: the offset counted lines back from the newest line, so the
    /// bottom was the reference point — and the bottom moves while a response
    /// streams. Scrolling back to re-read something meant watching it slide off
    /// the top at exactly the rate output arrived, because "twenty lines from the
    /// end" names different lines each time the end moves. This is the bug that
    /// made earlier output look permanently out of reach.
    #[test]
    fn a_scrolled_view_holds_its_place_while_output_streams() {
        let mut app = App::new();
        let body: String = (1..=40)
            .map(|n| format!("L{n:02}\n"))
            .collect::<Vec<_>>()
            .concat();
        let index = app.push_message(Message::new("ZAVORA", body));

        let mut terminal = Terminal::new(TestBackend::new(40, 12)).expect("terminal");
        let visible = |app: &App, terminal: &mut Terminal<TestBackend>| -> Vec<String> {
            terminal
                .draw(|frame| draw_transcript(frame, frame.area(), app))
                .expect("draw");
            let buffer = terminal.backend().buffer().clone();
            (0..buffer.area.height)
                .map(|row| {
                    (0..buffer.area.width)
                        .map(|col| buffer[(col, row)].symbol())
                        .collect::<String>()
                })
                .filter_map(|row| {
                    row.split_whitespace()
                        .find(|word| word.starts_with('L'))
                        .map(str::to_string)
                })
                .collect()
        };

        // Establish the bounds, then scroll back as a reader would mid-turn.
        visible(&app, &mut terminal);
        app.scroll_up(20);
        let before = visible(&app, &mut terminal);
        assert!(!before.is_empty(), "nothing on screen to compare");
        assert!(!app.follow_output, "scrolling up must detach from the tail");

        // Ten more lines arrive below the viewport.
        for n in 41..=50 {
            app.messages[index].text.push_str(&format!("L{n:02}\n"));
            app.messages[index].rendered.replace(None);
        }
        let after = visible(&app, &mut terminal);
        assert_eq!(
            before, after,
            "the view drifted while output streamed: was {before:?}, now {after:?}"
        );

        // Following still tracks the tail when it is armed.
        app.scroll_to_end();
        let tail = visible(&app, &mut terminal);
        assert!(
            tail.contains(&"L50".to_string()),
            "following should show the newest line, got {tail:?}"
        );
        for n in 51..=55 {
            app.messages[index].text.push_str(&format!("L{n:02}\n"));
            app.messages[index].rendered.replace(None);
        }
        let tail = visible(&app, &mut terminal);
        assert!(
            tail.contains(&"L55".to_string()),
            "a followed view must keep up with new output, got {tail:?}"
        );
    }

    /// The scroll indicator must not cost a content row.
    ///
    /// Regression: it was a top-edge block title, and ratatui reserves a row for
    /// one even when there is no top border, so the viewport silently shrank by a
    /// line the moment the view detached — a one-line scroll moved the view two
    /// lines. The indicator now sits on the bottom border, which is chrome
    /// already.
    #[test]
    fn the_scroll_indicator_does_not_steal_a_content_row() {
        let mut app = App::new();
        let body: String = (1..=40)
            .map(|n| format!("L{n:02}\n"))
            .collect::<Vec<_>>()
            .concat();
        app.push_message(Message::new("ZAVORA", body));

        let mut terminal = Terminal::new(TestBackend::new(40, 12)).expect("terminal");
        let content_rows = |app: &App, terminal: &mut Terminal<TestBackend>| -> Vec<String> {
            terminal
                .draw(|frame| draw_transcript(frame, frame.area(), app))
                .expect("draw");
            let buffer = terminal.backend().buffer().clone();
            (0..buffer.area.height)
                .map(|row| {
                    (0..buffer.area.width)
                        .map(|col| buffer[(col, row)].symbol())
                        .collect::<String>()
                })
                .filter_map(|row| {
                    row.split_whitespace()
                        .find(|word| word.starts_with('L'))
                        .map(str::to_string)
                })
                .collect()
        };

        let following = content_rows(&app, &mut terminal);
        app.scroll_up(1);
        let detached = content_rows(&app, &mut terminal);

        assert_eq!(
            following.len(),
            detached.len(),
            "the viewport changed height when the indicator appeared: \
             {following:?} then {detached:?}"
        );
        // One line up means exactly one line of movement.
        let first_before: usize = following[0]
            .trim_start_matches('L')
            .parse()
            .expect("number");
        let first_after: usize = detached[0].trim_start_matches('L').parse().expect("number");
        assert_eq!(
            first_before - first_after,
            1,
            "a one-line scroll moved {} lines: {following:?} then {detached:?}",
            first_before - first_after
        );

        // And the indicator is present, naming how far from the tail we are.
        terminal
            .draw(|frame| draw_transcript(frame, frame.area(), &app))
            .expect("draw");
        let buffer = terminal.backend().buffer().clone();
        let frame_text: String = (0..buffer.area.height)
            .map(|row| {
                (0..buffer.area.width)
                    .map(|col| buffer[(col, row)].symbol())
                    .collect::<String>()
            })
            .collect::<Vec<_>>()
            .join("\n");
        assert!(
            frame_text.contains("1 lines below"),
            "the indicator should say how far below the tail the view is:\n{frame_text}"
        );
    }

    /// The welcome screen starts under the header, not floating in the middle.
    ///
    /// Regression: it was centred vertically in the transcript pane, which put a
    /// bank of blank rows between the header and the first thing worth reading on
    /// a tall terminal. The transcript itself renders from the top of this pane,
    /// so the welcome now occupies the rows the first exchange will occupy.
    #[test]
    fn the_welcome_screen_starts_at_the_top_of_the_pane() {
        let app = App::new();
        assert!(app.messages.is_empty(), "this is the startup state");

        let mut terminal = Terminal::new(TestBackend::new(80, 30)).expect("terminal");
        terminal
            .draw(|frame| draw_transcript(frame, frame.area(), &app))
            .expect("draw");
        let buffer = terminal.backend().buffer().clone();
        let rows: Vec<String> = (0..buffer.area.height)
            .map(|row| {
                (0..buffer.area.width)
                    .map(|col| buffer[(col, row)].symbol())
                    .collect::<String>()
                    .trim()
                    .to_string()
            })
            .collect();

        let first_content = rows
            .iter()
            .position(|row| !row.is_empty())
            .expect("the welcome screen drew nothing");
        assert_eq!(
            first_content,
            0,
            "the welcome screen left {first_content} blank rows above it: {:?}",
            &rows[..=first_content]
        );
        assert!(
            rows[0].contains("ZAVORA"),
            "the first row should be the title, got {:?}",
            rows[0]
        );
    }

    /// Half a page keeps context across the jump; a whole page does not.
    #[test]
    fn a_half_page_step_is_half_the_viewport() {
        let app = App::new();
        app.viewport.set(40);
        assert_eq!(app.half_page_step(), 20);
        assert!(
            app.half_page_step() < app.page_step(),
            "the default step must be smaller than a whole page"
        );
        // Degenerate heights still make progress rather than stalling.
        app.viewport.set(1);
        assert_eq!(app.half_page_step(), 1);
        app.viewport.set(0);
        assert_eq!(app.half_page_step(), 1);
    }

    /// A transcript taller than `u16::MAX` stays reachable end to end.
    ///
    /// Regression: scroll state was `u16` and the renderer clamped the bound with
    /// `u16::try_from(..).unwrap_or(u16::MAX)`, so past 65535 rendered rows the
    /// offset saturated and the top of the conversation could not be reached at
    /// all. The window is now sliced in `usize` space instead of leaning on
    /// `Paragraph::scroll`, whose offset is a `u16`.
    #[test]
    fn a_transcript_taller_than_u16_stays_reachable() {
        let mut app = App::new();
        app.push_message(Message::new("YOU", "count for me"));
        // Distinct first and last lines, comfortably past the old ceiling.
        let body: String = (0..70_000)
            .map(|n| format!("row {n}\n"))
            .collect::<Vec<_>>()
            .concat();
        app.push_message(Message::new("ZAVORA", body));

        let mut terminal = Terminal::new(TestBackend::new(80, 24)).expect("terminal");
        let render = |app: &App, terminal: &mut Terminal<TestBackend>| -> String {
            terminal
                .draw(|frame| draw_transcript(frame, frame.area(), app))
                .expect("draw");
            let buffer = terminal.backend().buffer().clone();
            (0..buffer.area.height)
                .map(|row| {
                    (0..buffer.area.width)
                        .map(|col| buffer[(col, row)].symbol())
                        .collect::<String>()
                })
                .collect::<Vec<_>>()
                .join("\n")
        };

        // The first draw publishes the bound the key handler clamps against.
        let bottom = render(&app, &mut terminal);
        assert!(
            bottom.contains("row 69999"),
            "should start pinned to the newest output"
        );
        assert!(
            app.max_scroll.get() > u16::MAX as usize,
            "this transcript must exceed the old u16 ceiling, got {}",
            app.max_scroll.get()
        );

        app.scroll_to_start();
        let top = render(&app, &mut terminal);
        assert!(
            top.contains("count for me"),
            "the beginning of a very long conversation was unreachable:\n{}",
            top.lines().take(3).collect::<Vec<_>>().join("\n")
        );

        app.scroll_to_end();
        assert!(render(&app, &mut terminal).contains("row 69999"));
    }

    /// Message navigation lands on response boundaries, not fixed line counts.
    #[test]
    fn message_navigation_moves_between_responses() {
        let mut app = App::new();
        for turn in 0..5 {
            app.push_message(Message::new("YOU", format!("question {turn}")));
            // Deliberately uneven lengths: a fixed step could not track these.
            app.push_message(Message::new(
                "ZAVORA",
                format!("answer {turn}\n").repeat(turn * 7 + 3),
            ));
        }

        let mut terminal = Terminal::new(TestBackend::new(80, 12)).expect("terminal");
        terminal
            .draw(|frame| draw_transcript(frame, frame.area(), &app))
            .expect("draw");

        let boundaries = app.message_rows.borrow().clone();
        assert_eq!(
            boundaries.len(),
            app.messages.len(),
            "every message needs a boundary"
        );
        assert!(
            boundaries.windows(2).all(|pair| pair[0] < pair[1]),
            "boundaries must ascend: {boundaries:?}"
        );

        // From the bottom, stepping back must stop on successive boundaries
        // rather than skipping or standing still.
        app.scroll_to_end();
        let mut visited = Vec::new();
        for _ in 0..4 {
            app.scroll_to_message(true);
            visited.push(app.top_offset());
        }
        assert!(
            visited.windows(2).all(|pair| pair[0] > pair[1]),
            "each step back must move further up: {visited:?}"
        );
        assert!(
            visited.iter().all(|row| boundaries.contains(row)),
            "every stop must be a message boundary: {visited:?} in {boundaries:?}"
        );

        // Forward again returns to boundaries, and running off the end follows
        // output rather than sticking.
        app.scroll_to_message(false);
        assert!(boundaries.contains(&app.top_offset()));
        for _ in 0..20 {
            app.scroll_to_message(false);
        }
        assert!(
            app.follow_output,
            "running past the last response should resume following output"
        );

        // And running off the top lands at the very beginning.
        for _ in 0..20 {
            app.scroll_to_message(true);
        }
        assert_eq!(app.top_offset(), 0, "should come to rest at the first line");
    }

    /// Reaching the bottom resumes following streamed output.
    #[test]
    fn returning_to_the_bottom_resumes_following_output() {
        let mut app = App::new();
        app.max_scroll.set(30);
        app.viewport.set(20);

        app.scroll_up(10);
        assert!(!app.follow_output, "scrolling up should detach from output");

        app.scroll_down(10);
        assert_eq!(app.top_offset(), 30, "should be back at the tail");
        assert!(
            app.follow_output,
            "returning to the bottom should resume following"
        );
    }

    /// Prose fills the pane by default, and honours a cap when one is set.
    #[test]
    fn prose_fills_the_pane_by_default_and_caps_on_request() {
        fn widest_prose(prose_width: Option<u16>) -> usize {
            let mut app = App::new();
            app.prose_width = prose_width;
            app.push_message(Message::new("ZAVORA", "word ".repeat(200)));

            let mut terminal = Terminal::new(TestBackend::new(200, 16)).expect("terminal");
            // Render the transcript alone: `draw` also paints a header rule, a
            // composer border and the pane's own bottom border, all of which are
            // full-width chrome that would measure as content.
            terminal
                .draw(|frame| draw_transcript(frame, frame.area(), &app))
                .expect("draw");
            let buffer = terminal.backend().buffer().clone();
            (0..buffer.area.height.saturating_sub(1))
                .map(|row| {
                    (0..buffer.area.width.saturating_sub(1))
                        .map(|column| buffer[(column, row)].symbol())
                        .collect::<String>()
                        .trim_end()
                        .chars()
                        .count()
                })
                .max()
                .unwrap_or(0)
        }

        // Default: prose uses the width it is given.
        let filled = widest_prose(None);
        assert!(
            filled > 150,
            "prose should fill a 200-column pane, reached only {filled}"
        );

        // Capped: wraps well short of the pane, leaving the space `/width` asks for.
        let capped = widest_prose(Some(88));
        assert!(
            capped <= 92,
            "an 88-column cap should hold, reached {capped}"
        );
        assert!(capped < filled, "the cap made no difference");
    }

    /// Print both widths so the layout can be reviewed by eye.
    #[test]
    fn show_rendered_output() {
        for width in [80u16, 200u16] {
            eprintln!("\n===== {width} columns =====");
            eprintln!("{}", render(width, 34));
        }
    }
}
