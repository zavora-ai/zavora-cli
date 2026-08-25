//! Keyboard, mouse, composer, and prompt-submission handling.

use super::commands::{
    clipboard_payload, copy_to_clipboard, dispatch_tui_command, export_transcript,
    run_shell_command,
};
use super::render::centered;
use super::*;

pub(super) fn handle_mouse_click(app: &mut App, column: u16, row: u16, area: Rect) {
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
pub(super) fn apply_navigation(app: &mut App, action: crate::tui_keys::ActionId) -> bool {
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
pub(super) async fn handle_key(
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
pub(super) async fn submit_input(
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
                retrieval.clone(),
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

pub(super) fn previous_boundary(text: &str, cursor: usize) -> usize {
    text[..cursor]
        .char_indices()
        .next_back()
        .map_or(0, |(index, _)| index)
}

pub(super) fn next_boundary(text: &str, cursor: usize) -> usize {
    text[cursor..]
        .char_indices()
        .nth(1)
        .map_or(text.len(), |(index, _)| cursor + index)
}

pub(super) fn previous_word_boundary(text: &str, cursor: usize) -> usize {
    let prefix = &text[..cursor];
    let trimmed = prefix.trim_end_matches(char::is_whitespace);
    trimmed
        .char_indices()
        .rev()
        .find(|(_, character)| character.is_whitespace())
        .map_or(0, |(index, character)| index + character.len_utf8())
}

pub(super) fn next_word_boundary(text: &str, cursor: usize) -> usize {
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
