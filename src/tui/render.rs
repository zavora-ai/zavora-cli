//! Ratatui rendering and presentation-only Markdown formatting.

use super::*;

pub(super) fn draw(frame: &mut Frame<'_>, app: &App, cfg: &RuntimeConfig) {
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

pub(super) fn draw_header(frame: &mut Frame<'_>, area: Rect, app: &App, cfg: &RuntimeConfig) {
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

pub(super) fn draw_transcript(frame: &mut Frame<'_>, area: Rect, app: &App) {
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

pub(super) fn draw_welcome(frame: &mut Frame<'_>, area: Rect) {
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

pub(super) fn draw_activity(frame: &mut Frame<'_>, area: Rect, app: &App) {
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

pub(super) fn draw_composer(frame: &mut Frame<'_>, area: Rect, app: &App) {
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

pub(super) fn draw_footer(frame: &mut Frame<'_>, area: Rect, app: &App, cfg: &RuntimeConfig) {
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

pub(super) fn composer_height(app: &App, width: u16, height: u16) -> u16 {
    let inner = width.saturating_sub(4).max(1) as usize;
    let lines = app
        .input
        .split('\n')
        .map(|line| UnicodeWidthStr::width(line).max(1).div_ceil(inner))
        .sum::<usize>();
    (lines as u16 + 2).clamp(3, 8.min((height / 3).max(3)))
}

pub(super) fn visual_cursor(text: &str, width: u16) -> (u16, u16) {
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

pub(super) fn friendly_tool_name(name: &str) -> String {
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

pub(super) fn friendly_detail(detail: &str, width: usize) -> String {
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
pub(super) fn markdown_lines_wrapped(markdown: &str, width: usize) -> Vec<Line<'static>> {
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
pub(super) fn code_surface_width(pane: usize) -> usize {
    pane.saturating_sub(2).clamp(20, 100)
}

/// The language label that opens a code block.
pub(super) fn code_fence_label(language: &str, pane: usize) -> Line<'static> {
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
pub(super) fn code_block_lines(code: &[String], pane: usize) -> Vec<Line<'static>> {
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

pub(super) fn inline_spans(input: &str) -> Vec<Span<'static>> {
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

pub(super) fn code_line(raw: &str) -> Line<'static> {
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

pub(super) fn centered(width: u16, height: u16, area: Rect) -> Rect {
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

pub(super) fn format_mcp_markdown(
    cfg: &RuntimeConfig,
    runtime_tools: &ResolvedRuntimeTools,
) -> String {
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

pub(super) fn format_mcp_doctor_markdown(
    cfg: &RuntimeConfig,
    runtime_tools: &ResolvedRuntimeTools,
) -> String {
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

pub(super) fn format_agents_markdown() -> String {
    let mut output = String::from("## Agents\n\n");
    match crate::capabilities::CapabilitySnapshot::load(&[], &[]) {
        Ok(snapshot) if !snapshot.agents.is_empty() => {
            for agent in snapshot.agents {
                output.push_str(&format!(
                    "- **{}** — {}{} · {}\n",
                    agent.name,
                    agent.source,
                    if agent.coordinator_callable {
                        " · callable"
                    } else {
                        ""
                    },
                    if agent.description.is_empty() {
                        "No description"
                    } else {
                        &agent.description
                    }
                ));
                output.push_str(&format!(
                    "  - Tools `{}` / deny `{}` · skills `{}` / deny `{}` · bounds {} turns, {}s\n",
                    agent.tool_scope,
                    if agent.deny_tools.is_empty() {
                        "none".to_string()
                    } else {
                        agent.deny_tools.join(", ")
                    },
                    if agent.allow_skills.is_empty() {
                        "all".to_string()
                    } else {
                        agent.allow_skills.join(", ")
                    },
                    if agent.deny_skills.is_empty() {
                        "none".to_string()
                    } else {
                        agent.deny_skills.join(", ")
                    },
                    agent
                        .max_turns
                        .map(|value| value.to_string())
                        .unwrap_or_else(|| "default".to_string()),
                    agent
                        .timeout_secs
                        .map(|value| value.to_string())
                        .unwrap_or_else(|| "default".to_string()),
                ));
            }
        }
        Ok(_) => output.push_str("No agents are configured.\n"),
        Err(error) => output.push_str(&format!("Agent catalog could not be loaded: {error}\n")),
    }
    output.push_str("\nThe coordinator can call configured agents automatically. Use `/delegate @NAME TASK` for one isolated session or `/parallel @NAME @NAME TASK` for live parallel dispatch.");
    output
}

pub(super) fn draw_palette(frame: &mut Frame<'_>, area: Rect, app: &App) {
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

pub(super) fn draw_command_suggestions(frame: &mut Frame<'_>, composer: Rect, app: &App) {
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

pub(super) fn draw_approval(frame: &mut Frame<'_>, area: Rect, approval: &PendingApproval) {
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
            Span::styled("Agent ", Style::default().fg(MUTED)),
            Span::styled(
                &approval.agent,
                Style::default().fg(TEXT).add_modifier(Modifier::BOLD),
            ),
        ]),
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
