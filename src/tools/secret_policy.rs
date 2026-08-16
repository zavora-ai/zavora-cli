//! Single-source secret and path containment policy.
//!
//! Before this module existed, `fs_read` refused to read `.env` and anything
//! under `.git` or `.zavora`, while the read-only shell fast path happily
//! auto-approved `cat .env` because `cat` appears in `READONLY_COMMANDS`. Two
//! subsystems disagreed about the same secret. Every reader now consults this
//! module so the two cannot drift apart again.
//!
//! Requirements 7.4 and 7.5; Correctness Property 6.

use std::path::{Component, Path, PathBuf};

/// Path segments that are never readable, at any depth.
pub const DENIED_SEGMENTS: &[&str] = &[".git", ".zavora"];

/// File names that are never readable, wherever they appear.
pub const DENIED_FILE_NAMES: &[&str] =
    &[".env", ".env.local", ".env.development", ".env.production"];

/// Commands that print the process environment. These are never auto-approved,
/// in any confirmation mode, because their output is a credential dump that no
/// path check can contain.
pub const ENVIRONMENT_READING_COMMANDS: &[&str] =
    &["env", "printenv", "set", "export", "declare", "typeset"];

/// Why a path was refused. Callers render this in their own voice.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DenyReason {
    /// A path component matched [`DENIED_SEGMENTS`].
    Segment(String),
    /// The file name matched [`DENIED_FILE_NAMES`].
    FileName(String),
}

/// Result of scanning a shell command for containment violations.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct CommandScan {
    /// Path-like arguments that policy refuses.
    pub denied_paths: Vec<PathBuf>,
    /// True when the command itself dumps the environment.
    pub reads_environment: bool,
}

impl CommandScan {
    /// True when the command must not be auto-approved.
    pub fn is_contained(&self) -> bool {
        self.denied_paths.is_empty() && !self.reads_environment
    }
}

/// Check a single path against segment and file-name policy.
///
/// The path is compared component-wise rather than by substring so that
/// `notes/environment.md` is allowed while `config/.env` is refused.
pub fn deny_reason(path: &Path) -> Option<DenyReason> {
    for component in path.components() {
        let segment = component.as_os_str().to_string_lossy();
        if DENIED_SEGMENTS
            .iter()
            .any(|denied| segment.eq_ignore_ascii_case(denied))
        {
            return Some(DenyReason::Segment(segment.to_string()));
        }
    }

    if let Some(name) = path.file_name().and_then(|value| value.to_str())
        && DENIED_FILE_NAMES
            .iter()
            .any(|denied| name.eq_ignore_ascii_case(denied))
    {
        return Some(DenyReason::FileName(name.to_string()));
    }

    None
}

/// True when policy refuses this path.
pub fn is_denied_path(path: &Path) -> bool {
    deny_reason(path).is_some()
}

/// True when `argv0` names a command that prints the process environment.
///
/// The basename is compared so that `/usr/bin/printenv` is caught alongside
/// `printenv`.
pub fn command_reads_environment(argv0: &str) -> bool {
    let base = Path::new(argv0)
        .file_name()
        .and_then(|value| value.to_str())
        .unwrap_or(argv0);
    ENVIRONMENT_READING_COMMANDS
        .iter()
        .any(|denied| base.eq_ignore_ascii_case(denied))
}

/// Normalize a path lexically, resolving `.` and `..` without touching the
/// filesystem.
///
/// Lexical normalization is deliberate: the target may not exist yet, and
/// `canonicalize` would fail or follow a symlink. `..` is popped so that
/// `docs/../.env` cannot slip past the file-name check.
fn normalize_lexically(path: &Path) -> PathBuf {
    let mut out = PathBuf::new();
    for component in path.components() {
        match component {
            Component::CurDir => {}
            Component::ParentDir => {
                if !out.pop() {
                    out.push("..");
                }
            }
            other => out.push(other.as_os_str()),
        }
    }
    out
}

/// True when a token looks like a path rather than a flag or an operator.
fn is_path_like(token: &str) -> bool {
    if token.is_empty() || token.starts_with('-') {
        return false;
    }
    // Shell operators survive shlex splitting; they are not paths.
    !matches!(
        token,
        "|" | "||" | "&" | "&&" | ";" | ">" | ">>" | "<" | "<<" | "2>" | "2>&1"
    )
}

/// Scan a shell command for arguments the read policy refuses, and for the
/// command itself dumping the environment.
///
/// Every candidate is normalized before comparison, so `./.env`, `docs/../.env`
/// and `.env` are all recognized as the same refused file. Absolute paths are
/// checked as given.
pub fn scan_command(command: &str) -> CommandScan {
    let mut scan = CommandScan::default();

    let tokens = match shlex::split(command) {
        Some(tokens) => tokens,
        // Unparseable input is the security pipeline's problem, not ours. Report
        // nothing rather than guessing at token boundaries.
        None => return scan,
    };

    let mut expect_command_word = true;
    for token in tokens {
        if expect_command_word {
            // Leading `VAR=value` assignments precede the real command word.
            let is_assignment = token.split_once('=').is_some_and(|(name, _)| {
                !name.is_empty() && name.chars().all(|c| c.is_ascii_alphanumeric() || c == '_')
            });
            if is_assignment {
                continue;
            }
            if command_reads_environment(&token) {
                scan.reads_environment = true;
            }
            expect_command_word = false;
            continue;
        }

        // A new command word begins after a separator or pipe.
        if matches!(token.as_str(), "|" | "||" | "&&" | ";" | "&") {
            expect_command_word = true;
            continue;
        }

        if !is_path_like(&token) {
            continue;
        }

        let normalized = normalize_lexically(Path::new(&token));
        if is_denied_path(&normalized) {
            scan.denied_paths.push(normalized);
        }
    }

    scan
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn denied_file_names_are_refused_wherever_they_appear() {
        assert!(is_denied_path(Path::new(".env")));
        assert!(is_denied_path(Path::new("config/.env.production")));
        assert!(is_denied_path(Path::new("/srv/app/.env.local")));
    }

    #[test]
    fn denied_segments_are_refused_at_any_depth() {
        assert!(is_denied_path(Path::new(".git/config")));
        assert!(is_denied_path(Path::new("nested/.zavora/sessions.db")));
        assert_eq!(
            deny_reason(Path::new(".git/config")),
            Some(DenyReason::Segment(".git".to_string()))
        );
    }

    #[test]
    fn similar_names_are_still_readable() {
        assert!(!is_denied_path(Path::new("notes/environment.md")));
        assert!(!is_denied_path(Path::new("src/env.rs")));
        assert!(!is_denied_path(Path::new(".envrc")));
        assert!(!is_denied_path(Path::new("gitignore")));
    }

    #[test]
    fn traversal_cannot_hide_a_denied_file() {
        let scan = scan_command("cat docs/../.env");
        assert_eq!(scan.denied_paths.len(), 1, "{scan:?}");
        assert!(!scan.is_contained());
    }

    #[test]
    fn relative_prefix_cannot_hide_a_denied_file() {
        assert!(!scan_command("cat ./.env").is_contained());
        assert!(!scan_command("strings .env.production").is_contained());
        assert!(!scan_command("xxd .git/index").is_contained());
    }

    #[test]
    fn environment_dumping_commands_are_flagged() {
        assert!(scan_command("env").reads_environment);
        assert!(scan_command("printenv").reads_environment);
        assert!(scan_command("/usr/bin/printenv PATH").reads_environment);
        assert!(scan_command("env | grep KEY").reads_environment);
    }

    #[test]
    fn environment_dumping_is_caught_after_a_pipe() {
        let scan = scan_command("ls | printenv");
        assert!(scan.reads_environment, "{scan:?}");
    }

    #[test]
    fn leading_assignments_do_not_hide_the_command_word() {
        let scan = scan_command("FOO=bar printenv");
        assert!(scan.reads_environment, "{scan:?}");
    }

    #[test]
    fn ordinary_read_only_commands_are_contained() {
        assert!(scan_command("ls -la").is_contained());
        assert!(scan_command("cat README.md").is_contained());
        assert!(scan_command("git status").is_contained());
        assert!(scan_command("grep -rn TODO src/").is_contained());
        assert!(scan_command("wc -l src/main.rs").is_contained());
    }

    #[test]
    fn unparseable_input_reports_nothing_and_defers_to_the_pipeline() {
        let scan = scan_command("cat 'unterminated");
        assert!(scan.is_contained());
    }

    #[test]
    fn fs_read_and_the_shell_fast_path_agree() {
        // Property 6: every path fs_read refuses is also refused as a shell
        // argument. The two lists are the same constants by construction; this
        // asserts the shell scanner actually applies them.
        for name in DENIED_FILE_NAMES {
            let scan = scan_command(&format!("cat {name}"));
            assert!(
                !scan.is_contained(),
                "shell fast path allowed a file fs_read refuses: {name}"
            );
        }
        for segment in DENIED_SEGMENTS {
            let scan = scan_command(&format!("cat {segment}/anything"));
            assert!(
                !scan.is_contained(),
                "shell fast path allowed a segment fs_read refuses: {segment}"
            );
        }
    }
}
