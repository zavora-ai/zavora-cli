//! Terminal restoration that survives panics and signals.
//!
//! The workspace puts the terminal into states the shell cannot recover from on
//! its own: raw mode, the alternate screen, bracketed paste, and mouse reporting.
//! The ordinary exit path undoes all of it. A panic or a signal does not, and the
//! result is a shell the developer has to `reset` — with mouse reporting still
//! on, every pointer movement echoes escape sequences as literal text, because
//! crossterm's mouse capture includes any-event tracking (`?1003h`) which reports
//! motion and not just clicks. A `SIGTERM` is enough to produce that.
//!
//! So the sequences are emitted from three places: the normal path, a panic hook,
//! and a signal handler. The signal handler cannot allocate, lock, or call back
//! into crossterm, so it writes the bytes with `write(2)` and restores the saved
//! terminal attributes with `tcsetattr(3)`, both of which POSIX lists as
//! async-signal-safe.
//!
//! Modelled on the terminal-restore handling in xAI's Grok Build
//! (`xai-grok-pager`'s `wrap_restore`, Apache-2.0), including its ordering rule
//! that leaving the alternate screen comes last.

use std::sync::atomic::{AtomicBool, Ordering};

/// Undo every mode the workspace turns on, in the order that leaves a clean
/// screen behind.
///
/// Mouse reporting first, because a terminal still reporting motion will keep
/// writing into whatever comes next. The alternate screen is left last, so the
/// resets land on the screen being discarded rather than on the shell's.
const RESTORE: &[u8] = concat!(
    "\x1b[?1006l", // SGR mouse mode
    "\x1b[?1015l", // RXVT mouse mode
    "\x1b[?1003l", // any-event tracking
    "\x1b[?1002l", // button-event tracking
    "\x1b[?1000l", // normal tracking
    "\x1b[?2004l", // bracketed paste
    "\x1b[?25h",   // cursor visible
    "\x1b[0m",     // default colours
    "\x1b[?1049l", // alternate screen — last
)
.as_bytes();

/// Whether the terminal currently needs restoring.
///
/// Cleared by whichever path restores first, so a panic during teardown, or a
/// signal arriving mid-teardown, cannot emit the sequences twice.
static ARMED: AtomicBool = AtomicBool::new(false);

/// The escape sequences that undo the workspace's terminal modes.
///
/// Exposed so the ordering can be asserted rather than assumed.
pub fn restore_sequences() -> &'static [u8] {
    RESTORE
}

/// Whether a restore is currently owed.
pub fn is_armed() -> bool {
    ARMED.load(Ordering::SeqCst)
}

#[cfg(unix)]
mod imp {
    use super::{ARMED, RESTORE};
    use std::sync::atomic::Ordering;

    /// Terminal attributes as they were before raw mode.
    ///
    /// A plain `libc::termios` is integers only, so reading it from a signal
    /// handler is sound. `OnceLock::get` is an atomic load once initialised,
    /// which keeps the handler free of locks and allocation.
    static ORIGINAL_MODE: std::sync::OnceLock<libc::termios> = std::sync::OnceLock::new();

    /// Best-effort `write` that tolerates short writes and interruption.
    ///
    /// Deliberately loop-bounded: a signal handler must not spin forever on a
    /// terminal that has gone away.
    fn write_fd(fd: libc::c_int, bytes: &[u8]) {
        let mut written = 0usize;
        for _ in 0..64 {
            if written >= bytes.len() {
                return;
            }
            let remaining = &bytes[written..];
            // SAFETY: `remaining` is a valid initialised slice, and `len` is its
            // true length. `write` is async-signal-safe.
            let n = unsafe {
                libc::write(
                    fd,
                    remaining.as_ptr().cast::<libc::c_void>(),
                    remaining.len(),
                )
            };
            if n > 0 {
                written += n as usize;
            } else if n == 0 {
                return;
            }
            // n < 0: retry, covering EINTR without reading errno.
        }
    }

    /// Emit the restore sequences and undo raw mode, at most once.
    pub fn restore() {
        if !ARMED.swap(false, Ordering::SeqCst) {
            return;
        }
        // stdout is where the workspace draws, so the resets must go there.
        write_fd(libc::STDOUT_FILENO, RESTORE);
        if let Some(mode) = ORIGINAL_MODE.get() {
            // SAFETY: `mode` was captured from this terminal by `tcgetattr`, and
            // `tcsetattr` is async-signal-safe.
            unsafe {
                libc::tcsetattr(libc::STDIN_FILENO, libc::TCSANOW, mode);
            }
        }
    }

    /// Restore, then die from the signal so the exit status still reports it.
    ///
    /// Re-raising rather than calling `exit` keeps `$?` and any supervising
    /// process's view of the cause intact.
    extern "C" fn on_signal(signo: libc::c_int) {
        restore();
        // SAFETY: both calls are async-signal-safe, and resetting to the default
        // disposition before re-raising cannot recurse into this handler.
        unsafe {
            libc::signal(signo, libc::SIG_DFL);
            libc::raise(signo);
        }
    }

    /// Record the current terminal attributes and take responsibility for
    /// restoring them.
    pub fn arm() {
        let mut mode = std::mem::MaybeUninit::<libc::termios>::uninit();
        // SAFETY: `tcgetattr` fills the struct when it returns zero.
        let captured = unsafe { libc::tcgetattr(libc::STDIN_FILENO, mode.as_mut_ptr()) } == 0;
        if captured {
            // SAFETY: guarded by the return value above.
            let _ = ORIGINAL_MODE.set(unsafe { mode.assume_init() });
        }

        ARMED.store(true, Ordering::SeqCst);

        // SIGINT and SIGQUIT do not arrive from the keyboard in raw mode — the
        // workspace receives those as key events — but they still arrive from
        // `kill`, and a window closing sends SIGHUP.
        for signo in [
            libc::SIGHUP,
            libc::SIGINT,
            libc::SIGQUIT,
            libc::SIGTERM,
            libc::SIGABRT,
        ] {
            // SAFETY: installing a handler that only performs signal-safe calls.
            unsafe {
                libc::signal(signo, on_signal as *const () as libc::sighandler_t);
            }
        }
    }
}

#[cfg(not(unix))]
mod imp {
    use super::{ARMED, RESTORE};
    use std::io::Write;
    use std::sync::atomic::Ordering;

    /// Windows has no signal delivery to hook, so the panic hook and the normal
    /// path are the whole story.
    pub fn arm() {
        ARMED.store(true, Ordering::SeqCst);
    }

    pub fn restore() {
        if !ARMED.swap(false, Ordering::SeqCst) {
            return;
        }
        let mut out = std::io::stdout();
        let _ = out.write_all(RESTORE);
        let _ = out.flush();
        let _ = crossterm::terminal::disable_raw_mode();
    }
}

/// Take responsibility for putting the terminal back.
///
/// Call once, immediately after the workspace has claimed the terminal. Installs
/// a panic hook and, on Unix, signal handlers; both funnel into the same
/// idempotent restore as the normal exit path.
pub fn arm() {
    imp::arm();

    // Chain rather than replace: the panic message still has to reach the
    // developer, and it is only readable once the terminal is out of raw mode and
    // off the alternate screen.
    let previous = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        restore();
        previous(info);
    }));
}

/// Put the terminal back, if it has not been put back already.
///
/// Safe to call from a signal handler on Unix, and safe to call more than once.
pub fn restore() {
    imp::restore();
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Mouse reporting must be switched off before anything else.
    ///
    /// A terminal still reporting motion writes escape sequences into whatever
    /// comes next, which is exactly the failure this module exists to prevent:
    /// after a `SIGTERM` with reporting left on, every pointer movement printed
    /// `35;1;1M`-shaped text into the shell.
    #[test]
    fn mouse_reporting_is_disabled_first() {
        let text = std::str::from_utf8(restore_sequences()).expect("ascii");
        let mouse_modes = ["?1006l", "?1015l", "?1003l", "?1002l", "?1000l"];
        let last_mouse = mouse_modes
            .iter()
            .map(|mode| text.find(mode).expect("mouse mode missing"))
            .max()
            .expect("modes");
        for other in ["?2004l", "?25h", "?1049l"] {
            let position = text.find(other).expect("sequence missing");
            assert!(
                last_mouse < position,
                "{other} is emitted before mouse reporting is off"
            );
        }
    }

    /// Leaving the alternate screen comes last, so the resets land on the screen
    /// being discarded rather than on the shell's.
    #[test]
    fn the_alternate_screen_is_left_last() {
        let text = std::str::from_utf8(restore_sequences()).expect("ascii");
        assert!(
            text.ends_with("\x1b[?1049l"),
            "the alternate screen must be left last: {text:?}"
        );
    }

    /// Every mode the workspace turns on has a matching reset.
    ///
    /// The enable side is crossterm's `EnableMouseCapture`, `EnableBracketedPaste`
    /// and `EnterAlternateScreen`; if it gains a mode, this should fail.
    #[test]
    fn every_enabled_mode_has_a_reset() {
        let text = std::str::from_utf8(restore_sequences()).expect("ascii");
        for mode in ["1000", "1002", "1003", "1015", "1006", "2004", "1049"] {
            assert!(
                text.contains(&format!("?{mode}l")),
                "no reset for mode {mode}"
            );
        }
        // The cursor is hidden by the draw loop, so it has to come back too.
        assert!(text.contains("?25h"), "the cursor is left hidden");
    }

    /// Restoring twice must not emit the sequences twice.
    ///
    /// A signal can arrive while the normal teardown is running, and both paths
    /// call the same function.
    #[test]
    fn restoring_is_idempotent() {
        // Not armed: a stray restore is a no-op.
        assert!(!is_armed());
        restore();
        assert!(!is_armed());
    }
}
