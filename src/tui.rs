//! Full-screen terminal workspace for interactive Zavora sessions.

use std::io;
use std::sync::Arc;
use std::time::{Duration, Instant};

use adk_rust::prelude::Runner;
use anyhow::{Context, Result};
use crossterm::event::{
    self, DisableBracketedPaste, EnableBracketedPaste, Event as TerminalEvent, KeyCode, KeyEvent,
    KeyEventKind, KeyModifiers,
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

use crate::checkpoint::snapshot_session_events;
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

#[derive(Clone, Copy, PartialEq, Eq)]
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

struct Message {
    role: String,
    text: String,
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
    palette: bool,
    follow_output: bool,
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
            palette: false,
            follow_output: true,
        }
    }

    fn push_system(&mut self, text: impl Into<String>) {
        self.messages.push(Message {
            role: "ZAVORA".into(),
            text: text.into(),
        });
    }

    fn apply(&mut self, event: UiEvent) {
        match event {
            UiEvent::AgentChanged(agent) => self.active_agent = agent,
            UiEvent::TextDelta { author, text } => {
                let index = match self.current_assistant {
                    Some(index) if self.messages[index].role == author => index,
                    _ => {
                        self.messages.push(Message {
                            role: author,
                            text: String::new(),
                        });
                        let index = self.messages.len() - 1;
                        self.current_assistant = Some(index);
                        index
                    }
                };
                self.messages[index].text.push_str(&text);
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
            }
        }
    }
}

pub async fn run_tui_chat(
    mut cfg: RuntimeConfig,
    retrieval: Arc<dyn RetrievalService>,
    runtime_tools: ResolvedRuntimeTools,
    confirmation: ToolConfirmationSettings,
    telemetry: &TelemetrySink,
) -> Result<()> {
    let session_service = build_session_service(&cfg).await?;
    let (runner, provider, model) = build_single_runner_for_chat(
        &cfg,
        session_service.clone(),
        &runtime_tools,
        &confirmation,
        telemetry,
    )
    .await?;
    cfg.provider = provider;
    cfg.model = Some(model);
    let runner = Arc::new(runner);
    let telemetry = telemetry.clone();
    let (ui_tx, mut ui_rx) = tokio::sync::mpsc::unbounded_channel();
    let mut approval_rx = install_approval_bridge();

    enable_raw_mode().context("failed to enable terminal raw mode")?;
    let mut stdout = io::stdout();
    if let Err(error) = execute!(stdout, EnterAlternateScreen, EnableBracketedPaste) {
        disable_raw_mode().ok();
        return Err(error).context("failed to enter terminal workspace");
    }
    let backend = CrosstermBackend::new(stdout);
    let mut terminal = match ratatui::Terminal::new(backend) {
        Ok(terminal) => terminal,
        Err(error) => {
            let mut stdout = io::stdout();
            execute!(stdout, DisableBracketedPaste, LeaveAlternateScreen).ok();
            disable_raw_mode().ok();
            return Err(error).context("failed to initialize terminal workspace");
        }
    };
    let mut app = App::new();
    let mut last_context_refresh = Instant::now() - Duration::from_secs(1);
    let mut last_animation = Instant::now();
    let mut dirty = true;

    let result = async {
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
                });
                dirty = true;
            }
            if last_context_refresh.elapsed() >= Duration::from_millis(750) {
                if let Ok(events) = snapshot_session_events(&session_service, &cfg).await {
                    let usage =
                        compute_context_usage(&events, &provider.to_string(), &cfg.worker_model);
                    let context_percent = (usage.utilization() * 100.0).min(100.0) as u16;
                    if context_percent != app.context_percent {
                        app.context_percent = context_percent;
                        dirty = true;
                    }
                }
                last_context_refresh = Instant::now();
            }
            if app.busy && last_animation.elapsed() >= Duration::from_millis(250) {
                dirty = true;
                last_animation = Instant::now();
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
                            runner.clone(),
                            cfg.clone(),
                            retrieval.clone(),
                            telemetry.clone(),
                            ui_tx.clone(),
                        );
                        dirty = true;
                        if should_exit {
                            break;
                        }
                    }
                    TerminalEvent::Paste(text) if !app.busy && app.approval.is_none() => {
                        app.input.insert_str(app.cursor, &text);
                        app.cursor += text.len();
                        dirty = true;
                    }
                    TerminalEvent::Resize(_, _) => dirty = true,
                    _ => {}
                }
            }
        }
        Ok::<(), anyhow::Error>(())
    }
    .await;

    clear_approval_bridge();
    disable_raw_mode().ok();
    execute!(
        terminal.backend_mut(),
        DisableBracketedPaste,
        LeaveAlternateScreen
    )
    .ok();
    terminal.show_cursor().ok();
    result
}

#[allow(clippy::too_many_arguments)]
fn handle_key(
    key: KeyEvent,
    app: &mut App,
    runner: Arc<Runner>,
    cfg: RuntimeConfig,
    retrieval: Arc<dyn RetrievalService>,
    telemetry: TelemetrySink,
    tx: tokio::sync::mpsc::UnboundedSender<UiEvent>,
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
            app.approval = None;
        }
        return false;
    }
    if app.palette {
        if matches!(key.code, KeyCode::Esc | KeyCode::Char('q')) {
            app.palette = false;
        }
        return false;
    }
    match (key.code, key.modifiers) {
        (KeyCode::Char('c'), KeyModifiers::CONTROL) if !app.busy => return true,
        (KeyCode::Char('p'), KeyModifiers::CONTROL) => app.palette = true,
        (KeyCode::BackTab, _) => {
            app.mode = if app.mode == Mode::Build {
                Mode::Plan
            } else {
                Mode::Build
            };
        }
        (KeyCode::Esc, _) if app.busy => {
            runner.interrupt(&cfg.session_id);
            app.push_system("Cancellation requested.");
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
        (KeyCode::Left, _) => app.cursor = previous_boundary(&app.input, app.cursor),
        (KeyCode::Right, _) => app.cursor = next_boundary(&app.input, app.cursor),
        (KeyCode::Home, _) | (KeyCode::Char('a'), KeyModifiers::CONTROL) => app.cursor = 0,
        (KeyCode::End, _) | (KeyCode::Char('e'), KeyModifiers::CONTROL) => {
            app.cursor = app.input.len();
        }
        (KeyCode::Char('u'), KeyModifiers::CONTROL) => {
            app.input.clear();
            app.cursor = 0;
        }
        (KeyCode::Backspace, _) => {
            if app.cursor > 0 {
                let previous = previous_boundary(&app.input, app.cursor);
                app.input.drain(previous..app.cursor);
                app.cursor = previous;
            }
        }
        (KeyCode::Delete, _) if app.cursor < app.input.len() => {
            let next = next_boundary(&app.input, app.cursor);
            app.input.drain(app.cursor..next);
        }
        (KeyCode::Char('j'), KeyModifiers::CONTROL) => {
            app.input.insert(app.cursor, '\n');
            app.cursor += 1;
        }
        (KeyCode::Enter, _) if !app.busy => {
            let raw = app.input.trim().to_string();
            if raw.is_empty() {
                return false;
            }
            app.input.clear();
            app.cursor = 0;
            if raw == "/exit" {
                return true;
            }
            if raw == "/help" {
                app.palette = true;
                return false;
            }
            if raw == "/models" {
                app.push_system(format!(
                    "## Active model routes\n\n- **Worker:** {} / `{}`\n- **Planner:** {} / `{}`\n\nChoose different routes with the `--worker-provider`, `--worker-model`, `--planner-provider`, and `--planner-model` launch options.",
                    cfg.worker_provider, cfg.worker_model, cfg.planner_provider, cfg.planner_model
                ));
                return false;
            }
            if raw.starts_with('/') {
                app.push_system(
                    "This workspace currently handles `/help`, `/models`, and `/exit`. Start `ZAVORA_CLASSIC=1 zavora-cli chat` for the complete legacy slash-command shell.",
                );
                return false;
            }
            app.messages.push(Message {
                role: "YOU".into(),
                text: raw.clone(),
            });
            app.activities.clear();
            app.scroll = 0;
            app.follow_output = true;
            app.busy = true;
            app.active_agent = "starting".into();
            let prompt = if app.mode == Mode::Plan {
                format!(
                    "Planning mode. Use plan_work when it materially helps. Inspect and explain only; do not modify files.\n\n{raw}"
                )
            } else {
                raw
            };
            tokio::spawn(async move {
                if let Err(error) = enforce_prompt_limit(&prompt, cfg.max_prompt_chars) {
                    let _ = tx.send(UiEvent::Error(error.to_string()));
                    let _ = tx.send(UiEvent::Completed(String::new()));
                    return;
                }
                let prompt = match apply_guardrail(
                    &cfg,
                    &telemetry,
                    "input",
                    cfg.guardrail_input_mode,
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
                    max_chunks: cfg.retrieval_max_chunks,
                    max_chars: cfg.retrieval_max_chars,
                    min_score: cfg.retrieval_min_score,
                };
                let prompt = augment_prompt_with_retrieval(retrieval.as_ref(), &prompt, policy)
                    .unwrap_or(prompt);
                let result = if buffered_output_required(cfg.guardrail_output_mode) {
                    match run_prompt(&runner, &cfg, &prompt, &telemetry).await {
                        Ok(answer) => apply_guardrail(
                            &cfg,
                            &telemetry,
                            "output",
                            cfg.guardrail_output_mode,
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
                    run_prompt_to_ui(&runner, &cfg, &prompt, &telemetry, tx.clone())
                        .await
                        .map(|_| ())
                };
                if let Err(error) = result {
                    let _ = tx.send(UiEvent::Error(error.to_string()));
                    let _ = tx.send(UiEvent::Completed(String::new()));
                }
            });
        }
        (KeyCode::Char(character), modifiers) if !modifiers.contains(KeyModifiers::CONTROL) => {
            app.input.insert(app.cursor, character);
            app.cursor += character.len_utf8();
        }
        _ => {}
    }
    false
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
    if app.palette {
        draw_palette(frame, area);
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
            format!(" {} ", app.mode.label()),
            Style::default()
                .fg(Color::Black)
                .bg(if app.mode == Mode::Plan {
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
        lines.extend(markdown_lines(&message.text));
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
    let max_scroll = rendered_height.saturating_sub(visible) as u16;
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
            "Shift+Tab changes mode  ·  Ctrl+P opens actions",
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
    } else {
        " Ask Zavora "
    };
    let content = if app.input.is_empty() {
        Text::styled(
            "›  Describe the result you want…",
            Style::default().fg(MUTED),
        )
    } else {
        Text::styled(format!("›  {}", app.input), Style::default().fg(TEXT))
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
    if !app.busy && !app.palette && app.approval.is_none() {
        let before_cursor = format!("›  {}", &app.input[..app.cursor]);
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
            " {}  context {}%   planner {}                     Enter send   Ctrl+J newline   Ctrl+P actions ",
            app.mode.label(),
            app.context_percent,
            cfg.planner_model
        )
    } else {
        format!(
            " {}  context {}%                 Enter send   Ctrl+P actions ",
            app.mode.label(),
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

fn draw_palette(frame: &mut Frame<'_>, area: Rect) {
    let popup = centered(62, 60, area);
    frame.render_widget(Clear, popup);
    let lines = vec![
        Line::from(Span::styled(
            "Actions",
            Style::default().fg(ORANGE).add_modifier(Modifier::BOLD),
        )),
        Line::from(""),
        Line::from("Shift+Tab   Switch PLAN / BUILD"),
        Line::from("Esc         Stop the running agent"),
        Line::from("PageUp/Down Scroll conversation"),
        Line::from("Ctrl+J      Add a line to the prompt"),
        Line::from("Ctrl+End    Follow the newest response"),
        Line::from("/exit       Close the workspace"),
        Line::from("Ctrl+C      Exit when idle"),
        Line::from(""),
        Line::from(Span::styled(
            "Esc closes this palette",
            Style::default().fg(MUTED),
        )),
    ];
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
        ]),
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
}
