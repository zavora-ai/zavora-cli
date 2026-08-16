//! Full-screen terminal workspace for interactive Zavora sessions.

use std::io;
use std::sync::Arc;
use std::time::{Duration, Instant};

use adk_rust::prelude::{Content, Event, Runner};
use anyhow::{Context, Result};
use crossterm::event::{
    self, DisableBracketedPaste, DisableMouseCapture, EnableBracketedPaste, EnableMouseCapture,
    Event as TerminalEvent, KeyCode, KeyEvent, KeyEventKind, KeyModifiers, MouseButton,
    MouseEventKind,
};
use crossterm::execute;
use crossterm::terminal::{
    EnterAlternateScreen, LeaveAlternateScreen, disable_raw_mode, enable_raw_mode,
};
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
        usage: "/mouse",
        description: "Toggle mouse capture so you can select and copy with the mouse",
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

struct Message {
    role: String,
    text: String,
    /// Rendered lines, cached against the text length they were produced from.
    ///
    /// `draw_transcript` runs on every dirty frame — every streamed delta and
    /// every 250ms while busy — and previously re-parsed Markdown for the whole
    /// transcript each time, so redraw cost grew with session length. Caching
    /// makes it proportional to what changed. Requirement 4.6.
    rendered: std::cell::RefCell<Option<(usize, Vec<Line<'static>>)>>,
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

    /// Rendered lines, computed once per text revision.
    fn lines(&self) -> Vec<Line<'static>> {
        let mut cache = self.rendered.borrow_mut();
        if let Some((len, lines)) = cache.as_ref()
            && *len == self.text.len()
        {
            return lines.clone();
        }
        let lines = markdown_lines(&self.text);
        *cache = Some((self.text.len(), lines.clone()));
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
    scroll: u16,
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
    /// When on, the wheel scrolls the transcript — but the terminal's own
    /// click-drag text selection stops working, because the app receives the
    /// events instead. Toggling it off hands selection back so the developer can
    /// copy with the mouse the way they do everywhere else.
    mouse_capture: bool,
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
            scroll: 0,
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
            task_abort: None,
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
    let runtime_tools = Arc::new(runtime_tools);
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
    if let Err(error) = execute!(
        stdout,
        EnterAlternateScreen,
        EnableBracketedPaste,
        EnableMouseCapture
    ) {
        disable_raw_mode().ok();
        return Err(error).context("failed to enter terminal workspace");
    }
    let backend = CrosstermBackend::new(stdout);
    let mut terminal = match ratatui::Terminal::new(backend) {
        Ok(terminal) => terminal,
        Err(error) => {
            let mut stdout = io::stdout();
            execute!(
                stdout,
                DisableMouseCapture,
                DisableBracketedPaste,
                LeaveAlternateScreen
            )
            .ok();
            disable_raw_mode().ok();
            return Err(error).context("failed to initialize terminal workspace");
        }
    };
    let mut app = App::new();
    // Mirrors the terminal's actual mouse-reporting state so the loop only
    // issues a control sequence when it genuinely changes.
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
                                MouseEventKind::ScrollUp => {
                                    app.scroll = app.scroll.saturating_add(4);
                                    app.follow_output = false;
                                }
                                MouseEventKind::ScrollDown => {
                                    app.scroll = app.scroll.saturating_sub(4);
                                    app.follow_output = app.scroll == 0;
                                }
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
    disable_raw_mode().ok();
    execute!(
        terminal.backend_mut(),
        DisableMouseCapture,
        DisableBracketedPaste,
        LeaveAlternateScreen
    )
    .ok();
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
    match (key.code, key.modifiers) {
        (KeyCode::Char('c'), KeyModifiers::CONTROL) if !app.busy => return true,
        (KeyCode::Char('d'), KeyModifiers::CONTROL) if !app.busy && app.input.is_empty() => {
            return true;
        }
        (KeyCode::Char('p'), KeyModifiers::CONTROL) => app.palette = Some(PaletteState::default()),
        (KeyCode::Char('l'), KeyModifiers::CONTROL) if !app.busy => {
            app.messages.clear();
            app.activities.clear();
            app.scroll = 0;
            app.follow_output = true;
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
        (KeyCode::PageUp, _) => {
            app.scroll = app.scroll.saturating_add(8);
            app.follow_output = false;
        }
        (KeyCode::PageDown, _) => {
            app.scroll = app.scroll.saturating_sub(8);
            app.follow_output = app.scroll == 0;
        }
        (KeyCode::End, KeyModifiers::CONTROL) => {
            app.scroll = 0;
            app.follow_output = true;
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
        app.scroll = 0;
        app.follow_output = true;
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
    if raw == "/mouse" {
        app.mouse_capture = !app.mouse_capture;
        if app.mouse_capture {
            app.push_system(
                "Mouse capture **on** — the wheel scrolls the transcript, and click-drag \
                 selection is handled by the app.",
            );
        } else {
            app.push_system(
                "Mouse capture **off** — select and copy with the mouse as usual. \
                 Scroll with `PageUp`/`PageDown`. Run `/mouse` again to re-enable the wheel.",
            );
        }
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
    app.scroll = 0;
    app.follow_output = true;
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
                app.scroll = 0;
                app.follow_output = true;
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
        "## Workspace status\n\n- **Profile:** `{}`\n- **Agent:** `{}` ({})\n- **Mode:** {}\n- **Worker:** {} / `{}`\n- **Planner:** {} / `{}`\n- **Session:** `{}`\n- **Context:** {}%\n- **Auto-compact:** {}",
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
    if area.width >= 110 {
        let body = Layout::default()
            .direction(Direction::Horizontal)
            .constraints([Constraint::Percentage(70), Constraint::Percentage(30)])
            .split(root[1]);
        draw_transcript(frame, body[0], app);
        draw_activity(frame, body[1], app);
    } else {
        let activity_height = if app.activities.is_empty() && !app.busy {
            0
        } else if area.height < 22 {
            4
        } else {
            7
        };
        let body = Layout::default()
            .direction(Direction::Vertical)
            .constraints([Constraint::Min(5), Constraint::Length(activity_height)])
            .split(root[1]);
        draw_transcript(frame, body[0], app);
        if activity_height > 0 {
            draw_activity(frame, body[1], app);
        }
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
    if app.messages.is_empty() {
        draw_welcome(frame, area);
        return;
    }
    let mut lines = Vec::new();
    for message in &app.messages {
        let color = match message.role.as_str() {
            "YOU" => Color::Cyan,
            "ZAVORA" => ORANGE,
            _ => Color::Green,
        };
        let role = if message.role == "YOU" {
            " YOU "
        } else {
            " ZAVORA "
        };
        lines.push(Line::from(vec![
            Span::styled(
                role,
                Style::default()
                    .fg(Color::Black)
                    .bg(color)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::styled(
                if message.role == "YOU" {
                    "  request"
                } else {
                    "  response"
                },
                Style::default().fg(MUTED),
            ),
        ]));
        lines.push(Line::default());
        lines.extend(message.lines());
        lines.push(Line::default());
        lines.push(Line::from(Span::styled(
            "─".repeat(area.width.saturating_sub(5) as usize),
            Style::default().fg(Color::Rgb(48, 51, 59)),
        )));
        lines.push(Line::default());
    }
    let available = area.width.saturating_sub(4).max(1) as usize;
    let visible = area.height.saturating_sub(2) as usize;
    let rendered_height: usize = lines
        .iter()
        .map(|line| line.width().max(1).div_ceil(available))
        .sum();
    let max_scroll = u16::try_from(rendered_height.saturating_sub(visible)).unwrap_or(u16::MAX);
    let offset = max_scroll.saturating_sub(app.scroll.min(max_scroll));
    let title = if app.scroll > 0 {
        format!(
            " Conversation  ·  {} lines below ",
            app.scroll.min(max_scroll)
        )
    } else {
        " Conversation ".to_string()
    };
    frame.render_widget(
        Paragraph::new(Text::from(lines))
            .wrap(Wrap { trim: false })
            .scroll((offset, 0))
            .block(
                Block::default()
                    .title(title)
                    .borders(Borders::RIGHT | Borders::BOTTOM)
                    .border_style(Style::default().fg(Color::DarkGray)),
            ),
        area,
    );
}

fn draw_welcome(frame: &mut Frame<'_>, area: Rect) {
    let height = 12.min(area.height);
    let top = area.height.saturating_sub(height) / 2;
    let welcome_area = Rect::new(
        area.x.saturating_add(2),
        area.y.saturating_add(top),
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
            "Shift+Tab mode  ·  Ctrl+P actions  ·  ! direct shell  ·  /copy clipboard  ·  /mouse to select text",
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
    let status = if area.width >= 100 {
        format!(
            " {}  context {}%   planner {}             Enter send   ↑↓ history   Ctrl+P actions ",
            if app.shell_mode {
                "SHELL"
            } else {
                app.mode.label()
            },
            app.context_percent,
            cfg.planner_model
        )
    } else {
        format!(
            " {}  context {}%          Enter send   ↑↓ history   Ctrl+P actions ",
            if app.shell_mode {
                "SHELL"
            } else {
                app.mode.label()
            },
            app.context_percent
        )
    };
    frame.render_widget(
        Paragraph::new(status).style(Style::default().fg(MUTED)),
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

fn markdown_lines(markdown: &str) -> Vec<Line<'static>> {
    let mut lines = Vec::new();
    let mut code = false;
    for raw in markdown.lines() {
        if let Some(language) = raw.trim().strip_prefix("```") {
            code = !code;
            if code {
                lines.push(Line::from(Span::styled(
                    format!(
                        "  {}",
                        if language.is_empty() {
                            "code"
                        } else {
                            language
                        }
                    ),
                    Style::default()
                        .fg(ORANGE)
                        .bg(Color::Rgb(22, 24, 30))
                        .add_modifier(Modifier::BOLD),
                )));
            } else {
                lines.push(Line::default());
            }
            continue;
        }
        if code {
            lines.push(code_line(raw));
            continue;
        }
        if raw.is_empty() {
            lines.push(Line::default());
        } else if let Some(heading) = raw.strip_prefix("### ") {
            lines.push(Line::from(Span::styled(
                heading.to_string(),
                Style::default().fg(TEXT).add_modifier(Modifier::BOLD),
            )));
        } else if let Some(heading) = raw.strip_prefix("## ").or_else(|| raw.strip_prefix("# ")) {
            lines.push(Line::from(Span::styled(
                heading.to_string(),
                Style::default().fg(ORANGE).add_modifier(Modifier::BOLD),
            )));
        } else if let Some(item) = raw.strip_prefix("- ").or_else(|| raw.strip_prefix("* ")) {
            let mut spans = vec![Span::styled("  •  ", Style::default().fg(ORANGE))];
            spans.extend(inline_spans(item));
            lines.push(Line::from(spans));
        } else if let Some(quote) = raw.strip_prefix("> ") {
            let mut spans = vec![Span::styled("┃ ", Style::default().fg(Color::Cyan))];
            spans.extend(inline_spans(quote));
            lines.push(Line::from(spans).style(Style::default().fg(MUTED)));
        } else {
            lines.push(Line::from(inline_spans(raw)));
        }
    }
    if markdown.is_empty() {
        lines.push(Line::default());
    }
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

    /// Mouse capture starts on so the wheel scrolls, and toggles off so the
    /// terminal's own selection works.
    #[test]
    fn mouse_capture_starts_on_and_toggles() {
        let mut app = App::new();
        assert!(app.mouse_capture, "wheel scrolling should work by default");
        app.mouse_capture = !app.mouse_capture;
        assert!(!app.mouse_capture);
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

        let first = message.lines();
        assert!(!first.is_empty());
        // Cache populated at the current text length.
        assert_eq!(
            message.rendered.borrow().as_ref().map(|(len, _)| *len),
            Some(message.text.len())
        );

        // A second read must reuse the cache rather than reparse.
        let second = message.lines();
        assert_eq!(first.len(), second.len());

        // Appending invalidates it.
        message.append(" More text.");
        assert!(
            message.rendered.borrow().is_none(),
            "appending did not invalidate the render cache"
        );
        let third = message.lines();
        assert!(third.len() >= first.len());
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
        let lines =
            markdown_lines("## Example\nUse `cargo check`.\n```rust\nlet ready = true;\n```");
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
