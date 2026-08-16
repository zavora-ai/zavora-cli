//! Text measurement and wrapping for the terminal workspace.
//!
//! The workspace previously delegated wrapping to ratatui's `Paragraph`, which
//! has two limits that show up as poor typography:
//!
//! 1. It wraps to the full width of its pane. In the side-by-side layout a
//!    200-column terminal gives the transcript ~136 characters of prose per
//!    line, roughly double a comfortable reading measure. Long measure makes the
//!    eye lose its place on the return sweep, which reads as "wall of text" even
//!    when the spacing is correct.
//! 2. It has no concept of a hanging indent, so a wrapped list item returns to
//!    column zero and loses its alignment with the bullet above it.
//!
//! Wrapping here instead fixes both, and keeps the styling of each span intact
//! across the break. The font, weight, and letter spacing belong to the user's
//! terminal and are deliberately not our concern; measure, gutter, rhythm, and
//! indentation are.

use ratatui::style::Style;
use ratatui::text::{Line, Span};
use unicode_width::UnicodeWidthStr;

/// Comfortable maximum measure for prose, in columns.
///
/// Typographic convention puts the ideal around 66 characters and the usable
/// ceiling near 90. 88 leaves prose readable on a wide terminal without
/// stranding a narrow column of text in a large window.
pub const PROSE_MEASURE: usize = 88;

/// Columns of breathing room between the terminal edge and the text.
///
/// Text rendered flush against column zero is the most common reason a TUI looks
/// unfinished. Two columns is enough to read as intentional.
pub const GUTTER: usize = 2;

/// How a block of text should be laid out.
#[derive(Debug, Clone)]
pub struct WrapStyle {
    /// Total columns available for prefix plus text.
    pub width: usize,
    /// Prefix for the first line — a bullet, a quote bar, or nothing.
    pub first_prefix: Option<Span<'static>>,
    /// Prefix for continuation lines. Usually blanks matching the width of
    /// `first_prefix`, which is what produces a hanging indent.
    pub continuation_prefix: Option<Span<'static>>,
}

impl WrapStyle {
    /// A plain paragraph with no prefix.
    pub fn plain(width: usize) -> Self {
        Self {
            width,
            first_prefix: None,
            continuation_prefix: None,
        }
    }

    /// A marked block — list item, quote — whose continuation lines align under
    /// the text rather than under the marker.
    pub fn hanging(width: usize, marker: Span<'static>) -> Self {
        let indent = " ".repeat(marker.content.width());
        Self {
            width,
            first_prefix: Some(marker),
            continuation_prefix: Some(Span::raw(indent)),
        }
    }
}

/// One indivisible piece of styled text: either a word or a run of spaces.
struct Atom {
    text: String,
    style: Style,
    is_space: bool,
}

fn atomize(spans: &[Span<'static>]) -> Vec<Atom> {
    let mut atoms = Vec::new();
    for span in spans {
        let mut buffer = String::new();
        let mut buffer_is_space: Option<bool> = None;

        for character in span.content.chars() {
            let is_space = character == ' ' || character == '\t';
            match buffer_is_space {
                Some(current) if current == is_space => buffer.push(character),
                Some(_) => {
                    atoms.push(Atom {
                        text: std::mem::take(&mut buffer),
                        style: span.style,
                        is_space: buffer_is_space.unwrap_or(false),
                    });
                    buffer.push(character);
                    buffer_is_space = Some(is_space);
                }
                None => {
                    buffer.push(character);
                    buffer_is_space = Some(is_space);
                }
            }
        }

        if !buffer.is_empty() {
            atoms.push(Atom {
                text: buffer,
                style: span.style,
                is_space: buffer_is_space.unwrap_or(false),
            });
        }
    }
    atoms
}

/// Break a long word that cannot fit on a line of its own.
///
/// A URL or a hash has no break opportunity, so it is split at the column limit
/// rather than allowed to overflow the pane.
fn split_overlong(text: &str, limit: usize) -> (String, String) {
    if limit == 0 {
        return (String::new(), text.to_string());
    }
    let mut head = String::new();
    let mut used = 0usize;
    let mut chars = text.char_indices();
    for (index, character) in chars.by_ref() {
        let character_width = unicode_width::UnicodeWidthChar::width(character).unwrap_or(0);
        if used + character_width > limit {
            return (head, text[index..].to_string());
        }
        head.push(character);
        used += character_width;
    }
    (head, String::new())
}

/// Wrap styled spans into lines, preserving style across breaks.
pub fn wrap_styled(spans: &[Span<'static>], style: &WrapStyle) -> Vec<Line<'static>> {
    let first_prefix_width = style
        .first_prefix
        .as_ref()
        .map(|span| span.content.width())
        .unwrap_or(0);
    let continuation_prefix_width = style
        .continuation_prefix
        .as_ref()
        .map(|span| span.content.width())
        .unwrap_or(0);

    let atoms = atomize(spans);
    if atoms.is_empty() {
        let mut line_spans = Vec::new();
        if let Some(prefix) = style.first_prefix.clone() {
            line_spans.push(prefix);
        }
        return vec![Line::from(line_spans)];
    }

    let mut lines: Vec<Line<'static>> = Vec::new();
    let mut current: Vec<Span<'static>> = Vec::new();
    let mut used = 0usize;
    let mut first_line = true;
    let mut pending_spaces: Vec<Span<'static>> = Vec::new();

    // Available text columns, excluding the prefix.
    let text_width = |first: bool| -> usize {
        let prefix = if first {
            first_prefix_width
        } else {
            continuation_prefix_width
        };
        style.width.saturating_sub(prefix).max(1)
    };

    let flush = |lines: &mut Vec<Line<'static>>,
                 current: &mut Vec<Span<'static>>,
                 first_line: &mut bool| {
        let mut line_spans = Vec::new();
        let prefix = if *first_line {
            style.first_prefix.clone()
        } else {
            style.continuation_prefix.clone()
        };
        if let Some(prefix) = prefix {
            line_spans.push(prefix);
        }
        line_spans.append(current);
        lines.push(Line::from(line_spans));
        *first_line = false;
    };

    for atom in atoms {
        if atom.is_space {
            // Spaces are held back: a break discards them rather than leaving
            // trailing whitespace, which would otherwise show up as a ragged
            // right edge on selection and export.
            if !current.is_empty() {
                pending_spaces.push(Span::styled(atom.text, atom.style));
            }
            continue;
        }

        let limit = text_width(first_line);
        let pending_width: usize = pending_spaces.iter().map(|s| s.content.width()).sum();
        let word_width = atom.text.as_str().width();

        if !current.is_empty() && used + pending_width + word_width > limit {
            flush(&mut lines, &mut current, &mut first_line);
            used = 0;
            pending_spaces.clear();
        } else if !pending_spaces.is_empty() {
            used += pending_width;
            current.append(&mut pending_spaces);
        }

        // A word too long for an empty line is split rather than overflowed.
        let mut remaining = atom.text;
        loop {
            let limit = text_width(first_line);
            let remaining_width = remaining.as_str().width();
            if used + remaining_width <= limit {
                current.push(Span::styled(remaining, atom.style));
                used += remaining_width;
                break;
            }
            let (head, tail) = split_overlong(&remaining, limit.saturating_sub(used));
            if head.is_empty() {
                // Cannot fit anything on this line; break and retry.
                if current.is_empty() {
                    // Degenerate width; emit as-is to guarantee progress.
                    current.push(Span::styled(remaining, atom.style));
                    break;
                }
                flush(&mut lines, &mut current, &mut first_line);
                used = 0;
                continue;
            }
            current.push(Span::styled(head, atom.style));
            flush(&mut lines, &mut current, &mut first_line);
            used = 0;
            remaining = tail;
            if remaining.is_empty() {
                break;
            }
        }
    }

    if !current.is_empty() || lines.is_empty() {
        flush(&mut lines, &mut current, &mut first_line);
    }

    lines
}

/// The measure to use for prose given the pane width.
///
/// Caps at [`PROSE_MEASURE`] on wide terminals and yields the full width on
/// narrow ones, where every column matters more than the ideal measure does.
pub fn prose_measure(pane_width: usize) -> usize {
    // clamp is safe here: PROSE_MEASURE is a const well above 20.
    pane_width.clamp(20, PROSE_MEASURE)
}

#[cfg(test)]
mod tests {
    use super::*;
    use ratatui::style::{Modifier, Style};

    fn plain(text: &str) -> Vec<Span<'static>> {
        vec![Span::raw(text.to_string())]
    }

    fn rendered(lines: &[Line<'static>]) -> Vec<String> {
        lines
            .iter()
            .map(|line| {
                line.spans
                    .iter()
                    .map(|span| span.content.as_ref())
                    .collect::<String>()
            })
            .collect()
    }

    #[test]
    fn short_text_is_one_line() {
        let lines = wrap_styled(&plain("hello world"), &WrapStyle::plain(40));
        assert_eq!(rendered(&lines), vec!["hello world"]);
    }

    #[test]
    fn wrapping_breaks_at_word_boundaries_and_respects_the_measure() {
        let text = "the quick brown fox jumps over the lazy dog";
        let lines = wrap_styled(&plain(text), &WrapStyle::plain(20));
        for line in &rendered(&lines) {
            assert!(line.width() <= 20, "line exceeded measure: {line:?}");
        }
        // No word may be split when a break opportunity exists.
        let rejoined = rendered(&lines).join(" ");
        assert_eq!(rejoined, text);
    }

    #[test]
    fn no_line_carries_trailing_whitespace() {
        let lines = wrap_styled(
            &plain("alpha beta gamma delta epsilon"),
            &WrapStyle::plain(12),
        );
        for line in rendered(&lines) {
            assert_eq!(line.trim_end(), line, "trailing whitespace in {line:?}");
        }
    }

    /// The defect this module exists to fix: a wrapped bullet must stay aligned
    /// under its own text, not fall back to column zero.
    #[test]
    fn continuation_lines_hang_under_the_text() {
        let marker = Span::raw("  •  ".to_string());
        let style = WrapStyle::hanging(30, marker);
        let lines = wrap_styled(
            &plain("a list item long enough to wrap onto a second line"),
            &style,
        );
        let output = rendered(&lines);
        assert!(output.len() > 1, "expected a wrap: {output:?}");
        assert!(output[0].starts_with("  •  "));
        for continuation in &output[1..] {
            assert!(
                continuation.starts_with("     "),
                "continuation lost its hanging indent: {continuation:?}"
            );
            assert!(
                !continuation.trim_start().is_empty(),
                "continuation should carry text"
            );
        }
    }

    #[test]
    fn styles_survive_a_line_break() {
        let spans = vec![
            Span::raw("plain words here ".to_string()),
            Span::styled(
                "bold words here".to_string(),
                Style::default().add_modifier(Modifier::BOLD),
            ),
        ];
        let lines = wrap_styled(&spans, &WrapStyle::plain(18));
        let bold_survived = lines.iter().any(|line| {
            line.spans
                .iter()
                .any(|span| span.style.add_modifier.contains(Modifier::BOLD))
        });
        assert!(bold_survived, "bold styling was lost across the wrap");
    }

    #[test]
    fn an_unbreakable_word_is_split_rather_than_overflowing() {
        let url = "https://example.com/an/extremely/long/path/that/cannot/break";
        let lines = wrap_styled(&plain(url), &WrapStyle::plain(20));
        for line in rendered(&lines) {
            assert!(line.width() <= 20, "overflowed: {line:?}");
        }
        assert_eq!(rendered(&lines).concat(), url, "content was lost");
    }

    #[test]
    fn wide_characters_are_measured_by_display_width() {
        // CJK glyphs occupy two columns each.
        let lines = wrap_styled(&plain("日本語のテキストです"), &WrapStyle::plain(10));
        for line in rendered(&lines) {
            assert!(line.width() <= 10, "wide chars overflowed: {line:?}");
        }
    }

    #[test]
    fn empty_input_yields_one_line_with_its_prefix() {
        let lines = wrap_styled(&[], &WrapStyle::plain(20));
        assert_eq!(lines.len(), 1);

        let style = WrapStyle::hanging(20, Span::raw("┃ ".to_string()));
        let lines = wrap_styled(&[], &style);
        assert_eq!(rendered(&lines), vec!["┃ "]);
    }

    #[test]
    fn prose_measure_caps_wide_panes_and_yields_narrow_ones() {
        assert_eq!(prose_measure(200), PROSE_MEASURE);
        assert_eq!(prose_measure(60), 60);
        // Degenerate widths still leave something usable.
        assert_eq!(prose_measure(3), 20);
    }
}
