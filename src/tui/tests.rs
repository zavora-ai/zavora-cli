use super::commands::*;
use super::input::*;
use super::render::*;
use super::*;

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

    for spec in COMMAND_SPECS {
        for alias in spec.aliases {
            assert!(
                matching_commands(alias)
                    .iter()
                    .any(|candidate| candidate.name == spec.name),
                "/{alias} does not find {} in the palette",
                spec.name
            );
        }
    }
}

#[test]
fn slash_completion_prefers_exact_prefixes_and_preserves_argument_space() {
    let suggestions = slash_suggestions("/pla");
    assert_eq!(suggestions[0].name, "/planner-provider");
    assert!(command_input(suggestions[0]).ends_with(' '));

    let exact = slash_suggestions("/status");
    assert_eq!(exact.len(), 1);
    assert_eq!(command_input(exact[0]), "/status");

    for spec in COMMAND_SPECS {
        for alias in spec.aliases {
            let input = format!("/{alias}");
            let suggestion = slash_suggestions(&input)
                .into_iter()
                .find(|candidate| candidate.name == spec.name)
                .unwrap_or_else(|| panic!("/{alias} does not complete to {}", spec.name));
            assert!(
                command_input(suggestion).starts_with(spec.name),
                "/{alias} did not canonicalize to {}",
                spec.name
            );
        }
    }
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
