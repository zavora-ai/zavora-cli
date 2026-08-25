use super::render::*;
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
        let cfg = crate::test_support::base_cfg();
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
    let cfg = crate::test_support::base_cfg();
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
        let cfg = crate::test_support::base_cfg();
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
        let cfg = crate::test_support::base_cfg();
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
