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

use crate::interactive_commands::{COMMAND_SPECS, InteractiveCommandSpec as CommandSpec};

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
        let lines = render::markdown_lines_wrapped(&self.text, width);
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
    agent: String,
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

    fn sync_agent_runs(&mut self, runs: &[crate::agent_supervisor::AgentRun]) {
        for run in runs {
            let call_id = format!("agent-run:{}", run.id);
            let state = match run.status {
                crate::agent_supervisor::AgentRunStatus::Completed => ActivityState::Passed,
                crate::agent_supervisor::AgentRunStatus::Failed
                | crate::agent_supervisor::AgentRunStatus::Cancelled => ActivityState::Failed,
                _ => ActivityState::Running,
            };
            let elapsed_ms = run
                .started_at_ms
                .map(|started| {
                    run.completed_at_ms
                        .unwrap_or_else(|| crate::telemetry::unix_ms_now() as i64)
                        .saturating_sub(started)
                })
                .unwrap_or_default();
            let detail = format!(
                "{} · {} · {}",
                run.status.label(),
                run.id,
                run.workspace.display()
            );
            if let Some(activity) = self
                .activities
                .iter_mut()
                .find(|activity| activity.call_id.as_deref() == Some(&call_id))
            {
                activity.state = state;
                activity.detail = detail;
                activity.elapsed = run
                    .status
                    .is_terminal()
                    .then_some(Duration::from_millis(elapsed_ms.max(0) as u64));
            } else {
                let elapsed = Duration::from_millis(elapsed_ms.max(0) as u64);
                self.activities.push(Activity {
                    call_id: Some(call_id),
                    name: format!("agent:{}", run.agent),
                    detail,
                    state,
                    started: Instant::now()
                        .checked_sub(elapsed)
                        .unwrap_or_else(Instant::now),
                    elapsed: run.status.is_terminal().then_some(elapsed),
                });
            }
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
                "{} {} {} {} {}",
                spec.name,
                spec.usage,
                spec.description,
                spec.category,
                spec.aliases.join(" ")
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
        .filter(|spec| {
            spec.name.trim_start_matches('/').starts_with(&query)
                || spec.aliases.iter().any(|alias| alias.starts_with(&query))
        })
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
    let supervisor_store = crate::agent_supervisor::AgentRunStore::open_default()
        .await
        .ok();
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
                        agent: request.agent,
                        tool: request.tool,
                        detail: request.detail,
                        response: Some(request.response),
                        enables_agent_mode: false,
                    });
                    dirty = true;
                }
                if last_context_refresh.elapsed() >= Duration::from_millis(750) {
                    if let Some(store) = &supervisor_store
                        && let Ok(runs) = store.list(20).await
                    {
                        app.sync_agent_runs(&runs);
                        dirty = true;
                    }
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
                    let _ = terminal.draw(|frame| render::draw(frame, &app, &cfg));

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
                    terminal.draw(|frame| render::draw(frame, &app, &cfg))?;
                    dirty = false;
                }
                if event::poll(Duration::from_millis(40))? {
                    match event::read()? {
                        TerminalEvent::Key(key) if key.kind == KeyEventKind::Press => {
                            let should_exit = input::handle_key(
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
                                    input::handle_mouse_click(
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

mod commands;
mod input;
mod render;

#[cfg(test)]
mod tests;
#[cfg(test)]
mod typography_tests;
