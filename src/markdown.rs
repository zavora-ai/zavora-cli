/// Streaming markdown parser using winnow 0.7 + crossterm.
///
/// Ported from Q CLI's `parse.rs`, adapted for winnow 0.7 API.
/// Parses markdown token-by-token from a `Partial<&str>` buffer,
/// writing ANSI-styled output directly to a `Write` sink.
/// Returns `Incomplete` when more data is needed.
use std::io::Write;

use crossterm::Command;
use crossterm::style::{self, Attribute, Stylize};
use winnow::Partial;
use winnow::ascii::{self, digit1, space0, space1, till_line_ending};
use winnow::combinator::{alt, preceded, terminated};
use winnow::error::{ErrMode, ModalResult, ParserError};
use winnow::prelude::*;
use winnow::stream::{AsChar, Stream};
use winnow::token::{any, take_until, take_while};

// ---------------------------------------------------------------------------
// Error type
// ---------------------------------------------------------------------------

#[derive(Debug)]
pub enum MdError {
    Io(std::io::Error),
    Parse,
}

impl std::fmt::Display for MdError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            MdError::Io(e) => write!(f, "io: {e}"),
            MdError::Parse => write!(f, "parse error"),
        }
    }
}

impl<I: Stream> ParserError<I> for MdError {
    type Inner = Self;

    fn from_input(_input: &I) -> Self {
        Self::Parse
    }

    fn into_inner(self) -> Result<Self::Inner, Self> {
        Ok(self)
    }
}

// ---------------------------------------------------------------------------
// Parse state
// ---------------------------------------------------------------------------

pub struct ParseState {
    pub in_codeblock: bool,
    pub bold: bool,
    pub italic: bool,
    pub newline: bool,
    pub set_newline: bool,
    pub column: usize,
    pub terminal_width: Option<usize>,
}

impl Default for ParseState {
    fn default() -> Self {
        Self::new()
    }
}

impl ParseState {
    pub fn new() -> Self {
        let terminal_width = crossterm::terminal::size().ok().map(|(w, _)| w as usize);
        Self {
            in_codeblock: false,
            bold: false,
            italic: false,
            newline: true,
            set_newline: false,
            column: 0,
            terminal_width,
        }
    }
}

// ---------------------------------------------------------------------------
// Top-level parse step
// ---------------------------------------------------------------------------

/// Parse one markdown token from the input, writing styled output to `o`.
/// Returns the remaining input on success, or `Incomplete` if more data needed.
pub fn parse_markdown<'a>(
    mut i: Partial<&'a str>,
    mut o: impl Write,
    state: &mut ParseState,
) -> ModalResult<Partial<&'a str>, MdError> {
    let start = i.checkpoint();

    macro_rules! try_parser {
        ($($parser:ident),*) => {
            $({
                i.reset(&start);
                match $parser(&mut o, state).parse_next(&mut i) {
                    Err(ErrMode::Backtrack(_)) => {},
                    res => return res.map(|_| i),
                }
            })*
        };
    }

    if state.in_codeblock {
        try_parser!(codeblock_end, codeblock_line_ending, codeblock_fallback);
    } else {
        try_parser!(
            text,
            codeblock_begin,
            horizontal_rule,
            heading,
            bulleted_item,
            numbered_item,
            blockquote,
            code,
            bold,
            italic,
            line_ending,
            fallback
        );
    }

    Err(ErrMode::Backtrack(MdError::Parse))
}

// ---------------------------------------------------------------------------
// Parsers — normal mode
// ---------------------------------------------------------------------------

fn text<'a, 'b>(
    mut o: impl Write + 'b,
    state: &'b mut ParseState,
) -> impl FnMut(&mut Partial<&'a str>) -> ModalResult<(), MdError> + 'b {
    move |i| {
        let content = take_while(1.., |t| {
            AsChar::is_alphanum(t) || "+,.!?\"'/:;=@%&()[] ".contains(t)
        })
        .parse_next(i)?;
        advance(&mut o, state, content.len())?;
        q(&mut o, style::Print(content))
    }
}

fn heading<'a, 'b>(
    mut o: impl Write + 'b,
    state: &'b mut ParseState,
) -> impl FnMut(&mut Partial<&'a str>) -> ModalResult<(), MdError> + 'b {
    move |i| {
        if !state.newline {
            return Err(ErrMode::Backtrack(MdError::Parse));
        }
        let level = terminated(take_while(1.., |c| c == '#'), space1).parse_next(i)?;
        let print = format!("{level} ");
        advance(&mut o, state, print.len())?;
        q(&mut o, style::SetForegroundColor(style::Color::Magenta))?;
        q(&mut o, style::SetAttribute(Attribute::Bold))?;
        q(&mut o, style::Print(print))
    }
}

fn bulleted_item<'a, 'b>(
    mut o: impl Write + 'b,
    state: &'b mut ParseState,
) -> impl FnMut(&mut Partial<&'a str>) -> ModalResult<(), MdError> + 'b {
    move |i| {
        if !state.newline {
            return Err(ErrMode::Backtrack(MdError::Parse));
        }
        let ws = (space0, alt(("-", "*")), space1).parse_next(i)?.0;
        let print = format!("{ws}• ");
        advance(&mut o, state, print.len())?;
        q(&mut o, style::Print(print))
    }
}

fn numbered_item<'a, 'b>(
    mut o: impl Write + 'b,
    state: &'b mut ParseState,
) -> impl FnMut(&mut Partial<&'a str>) -> ModalResult<(), MdError> + 'b {
    move |i| {
        if !state.newline {
            return Err(ErrMode::Backtrack(MdError::Parse));
        }
        let (ws, digits, _, _) = (space0, digit1, ".", space1).parse_next(i)?;
        let print = format!("{ws}{digits}. ");
        advance(&mut o, state, print.len())?;
        q(&mut o, style::Print(print))
    }
}

fn horizontal_rule<'a, 'b>(
    mut o: impl Write + 'b,
    state: &'b mut ParseState,
) -> impl FnMut(&mut Partial<&'a str>) -> ModalResult<(), MdError> + 'b {
    move |i| {
        if !state.newline {
            return Err(ErrMode::Backtrack(MdError::Parse));
        }
        (
            space0,
            alt((
                take_while(3.., '-'),
                take_while(3.., '*'),
                take_while(3.., '_'),
            )),
        )
            .parse_next(i)?;
        state.set_newline = true;
        q(
            &mut o,
            style::Print(format!("{}\n", "━".repeat(40).dark_grey())),
        )
    }
}

fn code<'a, 'b>(
    mut o: impl Write + 'b,
    _state: &'b mut ParseState,
) -> impl FnMut(&mut Partial<&'a str>) -> ModalResult<(), MdError> + 'b {
    move |i| {
        "`".parse_next(i)?;
        let content = terminated(take_until(0.., "`"), "`").parse_next(i)?;
        q(&mut o, style::SetForegroundColor(style::Color::Green))?;
        q(&mut o, style::Print(content))?;
        q(&mut o, style::ResetColor)
    }
}

fn bold<'a, 'b>(
    mut o: impl Write + 'b,
    state: &'b mut ParseState,
) -> impl FnMut(&mut Partial<&'a str>) -> ModalResult<(), MdError> + 'b {
    move |i| {
        alt(("**", "__")).parse_next(i)?;
        state.bold = !state.bold;
        if state.bold {
            q(&mut o, style::SetAttribute(Attribute::Bold))
        } else {
            q(&mut o, style::SetAttribute(Attribute::NormalIntensity))
        }
    }
}

fn italic<'a, 'b>(
    mut o: impl Write + 'b,
    state: &'b mut ParseState,
) -> impl FnMut(&mut Partial<&'a str>) -> ModalResult<(), MdError> + 'b {
    move |i| {
        alt(("*", "_")).parse_next(i)?;
        state.italic = !state.italic;
        if state.italic {
            q(&mut o, style::SetAttribute(Attribute::Italic))
        } else {
            q(&mut o, style::SetAttribute(Attribute::NoItalic))
        }
    }
}

fn blockquote<'a, 'b>(
    mut o: impl Write + 'b,
    state: &'b mut ParseState,
) -> impl FnMut(&mut Partial<&'a str>) -> ModalResult<(), MdError> + 'b {
    move |i| {
        if !state.newline {
            return Err(ErrMode::Backtrack(MdError::Parse));
        }
        (">", space0).parse_next(i)?;
        let print = "│ ";
        advance(&mut o, state, print.len())?;
        q(&mut o, style::SetForegroundColor(style::Color::DarkGrey))?;
        q(&mut o, style::Print(print))
    }
}

fn line_ending<'a, 'b>(
    mut o: impl Write + 'b,
    state: &'b mut ParseState,
) -> impl FnMut(&mut Partial<&'a str>) -> ModalResult<(), MdError> + 'b {
    move |i| {
        ascii::line_ending.parse_next(i)?;
        state.column = 0;
        state.set_newline = true;
        q(&mut o, style::ResetColor)?;
        q(&mut o, style::SetAttribute(Attribute::Reset))?;
        q(&mut o, style::Print("\n"))
    }
}

fn fallback<'a, 'b>(
    mut o: impl Write + 'b,
    state: &'b mut ParseState,
) -> impl FnMut(&mut Partial<&'a str>) -> ModalResult<(), MdError> + 'b {
    move |i| {
        let c = any.parse_next(i)?;
        advance(&mut o, state, 1)?;
        if c != ' ' || state.column != 1 {
            q(&mut o, style::Print(c))
        } else {
            Ok(())
        }
    }
}

// ---------------------------------------------------------------------------
// Parsers — codeblock mode
// ---------------------------------------------------------------------------

fn codeblock_begin<'a, 'b>(
    mut o: impl Write + 'b,
    state: &'b mut ParseState,
) -> impl FnMut(&mut Partial<&'a str>) -> ModalResult<(), MdError> + 'b {
    move |i| {
        if !state.newline {
            return Err(ErrMode::Backtrack(MdError::Parse));
        }
        let language = preceded("```", till_line_ending).parse_next(i)?;
        ascii::line_ending.parse_next(i)?;
        state.in_codeblock = true;
        if !language.is_empty() {
            q(&mut o, style::Print(format!("{}\n", language.bold())))?;
        }
        q(&mut o, style::SetForegroundColor(style::Color::Green))
    }
}

fn codeblock_end<'a, 'b>(
    mut o: impl Write + 'b,
    state: &'b mut ParseState,
) -> impl FnMut(&mut Partial<&'a str>) -> ModalResult<(), MdError> + 'b {
    move |i| {
        "```".parse_next(i)?;
        state.in_codeblock = false;
        q(&mut o, style::ResetColor)
    }
}

fn codeblock_line_ending<'a, 'b>(
    mut o: impl Write + 'b,
    _state: &'b mut ParseState,
) -> impl FnMut(&mut Partial<&'a str>) -> ModalResult<(), MdError> + 'b {
    move |i| {
        ascii::line_ending.parse_next(i)?;
        q(&mut o, style::Print("\n"))
    }
}

fn codeblock_fallback<'a, 'b>(
    mut o: impl Write + 'b,
    _state: &'b mut ParseState,
) -> impl FnMut(&mut Partial<&'a str>) -> ModalResult<(), MdError> + 'b {
    move |i| {
        let c = any.parse_next(i)?;
        q(&mut o, style::Print(c))
    }
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// Track column position and emit a newline if we'd overflow the terminal width.
fn advance(
    mut o: impl Write,
    state: &mut ParseState,
    width: usize,
) -> Result<(), ErrMode<MdError>> {
    if let Some(tw) = state.terminal_width {
        if state.column > 0 && state.column + width > tw {
            state.column = width;
            return q(&mut o, style::Print('\n'));
        }
    }
    state.column += width;
    Ok(())
}

fn q(mut o: impl Write, cmd: impl Command) -> Result<(), ErrMode<MdError>> {
    use crossterm::QueueableCommand;
    o.queue(cmd).map_err(|e| ErrMode::Cut(MdError::Io(e)))?;
    Ok(())
}
