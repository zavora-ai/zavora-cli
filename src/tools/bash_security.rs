//! Bash command security validation pipeline.
//!
//! Each validator receives a `ValidationContext` and returns a `SecurityResult`.
//! The pipeline short-circuits on Allow/Deny, collects Ask reasons, and falls
//! through on Passthrough.

/// Result of a single security check.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SecurityResult {
    Allow(String),
    Deny(String),
    Ask(String),
    Passthrough,
}

/// Pre-processed command context for validators.
pub struct ValidationContext {
    pub original: String,
    pub base_command: String,
    /// Single quotes stripped.
    pub unquoted: String,
    /// All quotes stripped, safe redirections stripped.
    pub fully_unquoted: String,
    /// All quotes stripped, BEFORE safe-redirection stripping.
    pub fully_unquoted_pre_strip: String,
    /// Quote content stripped but quote chars ('/"") preserved.
    pub unquoted_keep_quotes: String,
}

// ---------------------------------------------------------------------------
// Context construction
// ---------------------------------------------------------------------------

pub fn build_context(command: &str) -> ValidationContext {
    let original = command.to_string();
    let base_command = command
        .split_whitespace()
        .next()
        .unwrap_or("")
        .to_string();
    let extraction = extract_quoted_content(command);
    let fully_unquoted_pre_strip = extraction.fully_unquoted.clone();
    let fully_unquoted = strip_safe_redirections(&extraction.fully_unquoted);
    ValidationContext {
        original,
        base_command,
        unquoted: extraction.single_stripped,
        fully_unquoted,
        fully_unquoted_pre_strip,
        unquoted_keep_quotes: extraction.keep_quote_chars,
    }
}

struct QuoteExtraction {
    single_stripped: String,
    fully_unquoted: String,
    keep_quote_chars: String,
}

fn extract_quoted_content(command: &str) -> QuoteExtraction {
    let mut single_stripped = String::new();
    let mut fully_unquoted = String::new();
    let mut keep_quote_chars = String::new();
    let mut in_single = false;
    let mut in_double = false;
    let mut escaped = false;

    for ch in command.chars() {
        if escaped {
            escaped = false;
            if !in_single { single_stripped.push(ch); }
            if !in_single && !in_double { fully_unquoted.push(ch); keep_quote_chars.push(ch); }
            continue;
        }
        if ch == '\\' && !in_single {
            escaped = true;
            if !in_single { single_stripped.push(ch); }
            if !in_single && !in_double { fully_unquoted.push(ch); keep_quote_chars.push(ch); }
            continue;
        }
        if ch == '\'' && !in_double {
            in_single = !in_single;
            keep_quote_chars.push(ch);
            continue;
        }
        if ch == '"' && !in_single {
            in_double = !in_double;
            keep_quote_chars.push(ch);
            continue;
        }
        if !in_single { single_stripped.push(ch); }
        if !in_single && !in_double { fully_unquoted.push(ch); keep_quote_chars.push(ch); }
    }

    QuoteExtraction { single_stripped, fully_unquoted, keep_quote_chars }
}

/// Strip safe redirections: 2>&1, N>/dev/null, </dev/null.
/// SECURITY: Each pattern MUST have trailing boundary (\s|$) to prevent prefix matching.
fn strip_safe_redirections(content: &str) -> String {
    let mut s = content.to_string();
    // Order matters: strip 2>&1 first, then >/dev/null, then </dev/null
    s = regex_replace(&s, r"\s+2\s*>&\s*1(?:\s|$)", " ");
    s = regex_replace(&s, r"[012]?\s*>\s*/dev/null(?:\s|$)", " ");
    s = regex_replace(&s, r"\s*<\s*/dev/null(?:\s|$)", " ");
    s
}

fn regex_replace(input: &str, pattern: &str, replacement: &str) -> String {
    // Simple regex-free implementation for the specific patterns we need
    let mut result = input.to_string();
    // For our specific patterns, we do iterative string matching
    for pat in expand_pattern(pattern) {
        while let Some(pos) = result.find(&pat) {
            let end = pos + pat.len();
            // Check trailing boundary: must be at end or followed by whitespace
            if end >= result.len() || result.as_bytes().get(end).map_or(true, |b| b.is_ascii_whitespace()) {
                result = format!("{}{}{}", &result[..pos], replacement, &result[end..]);
            } else {
                break;
            }
        }
    }
    result
}

/// Expand our specific redirect patterns into literal strings to match.
fn expand_pattern(pattern: &str) -> Vec<String> {
    // We only need to handle our three specific patterns
    if pattern.contains("2>&1") {
        vec![" 2>&1".to_string(), " 2>& 1".to_string()]
    } else if pattern.contains(">/dev/null") {
        vec![
            ">/dev/null".to_string(),
            "> /dev/null".to_string(),
            "1>/dev/null".to_string(),
            "1> /dev/null".to_string(),
            "2>/dev/null".to_string(),
            "2> /dev/null".to_string(),
        ]
    } else if pattern.contains("</dev/null") {
        vec!["</dev/null".to_string(), "< /dev/null".to_string()]
    } else {
        vec![]
    }
}

// ---------------------------------------------------------------------------
// Pipeline
// ---------------------------------------------------------------------------

type Validator = fn(&ValidationContext) -> SecurityResult;

const VALIDATORS: &[Validator] = &[
    validate_empty,
    validate_incomplete_commands,
    validate_zsh_dangerous_commands,
    validate_command_substitution,
    validate_shell_metacharacters,
    validate_dangerous_variables,
    validate_newlines,
    validate_redirections,
    validate_obfuscated_flags,
    validate_brace_expansion,
    validate_unicode_whitespace,
    validate_carriage_return,
    validate_proc_environ_access,
    validate_ifs_injection,
    validate_backslash_escaped_operators,
    validate_comment_quote_desync,
    validate_mid_word_hash,
    validate_malformed_token_injection,
    validate_jq_system_function,
    validate_git_commit_substitution,
];

/// Run the full security validation pipeline on a command.
pub fn validate_bash_command(command: &str) -> SecurityResult {
    let ctx = build_context(command);
    let mut ask_reasons = Vec::new();

    for validator in VALIDATORS {
        match validator(&ctx) {
            SecurityResult::Allow(reason) => return SecurityResult::Allow(reason),
            SecurityResult::Deny(reason) => return SecurityResult::Deny(reason),
            SecurityResult::Ask(reason) => ask_reasons.push(reason),
            SecurityResult::Passthrough => {}
        }
    }

    if !ask_reasons.is_empty() {
        return SecurityResult::Ask(ask_reasons.join("; "));
    }
    SecurityResult::Passthrough
}

// ---------------------------------------------------------------------------
// Validators
// ---------------------------------------------------------------------------

fn validate_empty(ctx: &ValidationContext) -> SecurityResult {
    if ctx.original.trim().is_empty() {
        SecurityResult::Allow("empty command".into())
    } else {
        SecurityResult::Passthrough
    }
}

fn validate_incomplete_commands(ctx: &ValidationContext) -> SecurityResult {
    let trimmed = ctx.original.trim();
    if trimmed.starts_with('\t') {
        return SecurityResult::Deny("starts with tab (incomplete fragment)".into());
    }
    if trimmed.starts_with('-') {
        return SecurityResult::Deny("starts with flags (incomplete fragment)".into());
    }
    if trimmed.starts_with("&&")
        || trimmed.starts_with("||")
        || trimmed.starts_with(';')
        || trimmed.starts_with(">>")
        || trimmed.starts_with('>')
        || trimmed.starts_with('<')
    {
        return SecurityResult::Deny("starts with operator (continuation line)".into());
    }
    SecurityResult::Passthrough
}

const ZSH_DANGEROUS: &[&str] = &[
    "zmodload", "emulate", "sysopen", "sysread", "syswrite", "sysseek",
    "zpty", "ztcp", "zsocket", "mapfile",
    "zf_rm", "zf_mv", "zf_ln", "zf_chmod", "zf_chown", "zf_mkdir", "zf_rmdir", "zf_chgrp",
];

fn validate_zsh_dangerous_commands(ctx: &ValidationContext) -> SecurityResult {
    if ZSH_DANGEROUS.contains(&ctx.base_command.as_str()) {
        return SecurityResult::Deny(format!("Zsh dangerous command: {}", ctx.base_command));
    }
    SecurityResult::Passthrough
}

const SUBSTITUTION_PATTERNS: &[(&str, &str)] = &[
    ("$(", "$() command substitution"),
    ("${", "${} parameter substitution"),
    ("$[", "$[] arithmetic expansion"),
    ("<(", "process substitution <()"),
    (">(", "process substitution >()"),
    ("=(", "Zsh process substitution =()"),
];

fn validate_command_substitution(ctx: &ValidationContext) -> SecurityResult {
    let content = &ctx.fully_unquoted;
    for &(pat, desc) in SUBSTITUTION_PATTERNS {
        if content.contains(pat) {
            return SecurityResult::Deny(format!("contains {}", desc));
        }
    }
    // Check for unescaped backticks
    if has_unescaped_char(content, '`') {
        return SecurityResult::Deny("contains backtick command substitution".into());
    }
    SecurityResult::Passthrough
}

fn validate_shell_metacharacters(ctx: &ValidationContext) -> SecurityResult {
    let content = &ctx.fully_unquoted;
    // Check for unescaped pipe, background, semicolon
    if has_unescaped_char(content, '|') {
        return SecurityResult::Ask("contains pipe operator".into());
    }
    // & but not && (which is caught by validate_incomplete if at start)
    // Check for standalone & (backgrounding)
    if content.contains(" & ") || content.ends_with(" &") || content.ends_with('&') {
        // But allow && (logical AND)
        let no_and = content.replace("&&", "");
        if no_and.contains('&') {
            return SecurityResult::Ask("contains background operator &".into());
        }
    }
    if has_unescaped_char(content, ';') {
        return SecurityResult::Ask("contains semicolon (command chaining)".into());
    }
    SecurityResult::Passthrough
}

fn validate_dangerous_variables(ctx: &ValidationContext) -> SecurityResult {
    let content = &ctx.fully_unquoted;
    for var in &["IFS", "PATH", "LD_PRELOAD", "LD_LIBRARY_PATH"] {
        // Match word boundary: VAR= at start or after whitespace
        let assign = format!("{}=", var);
        if content.starts_with(&assign)
            || content.contains(&format!(" {}", assign))
            || content.contains(&format!("\t{}", assign))
        {
            return SecurityResult::Deny(format!("dangerous variable assignment: {}=", var));
        }
    }
    SecurityResult::Passthrough
}

fn validate_newlines(ctx: &ValidationContext) -> SecurityResult {
    if ctx.original.contains('\n') {
        return SecurityResult::Deny("contains literal newline".into());
    }
    SecurityResult::Passthrough
}

fn validate_redirections(ctx: &ValidationContext) -> SecurityResult {
    let content = &ctx.fully_unquoted;
    // After stripping safe redirections, check for remaining > or >>
    if has_unescaped_char(content, '>') {
        return SecurityResult::Ask("contains output redirection".into());
    }
    // Input redirection (< but not <<)
    if content.contains('<') && !content.contains("<<") {
        return SecurityResult::Ask("contains input redirection".into());
    }
    SecurityResult::Passthrough
}

fn validate_obfuscated_flags(ctx: &ValidationContext) -> SecurityResult {
    // Check for flag-like tokens with non-ASCII or shell metacharacters
    for word in ctx.fully_unquoted.split_whitespace() {
        if word.starts_with('-') && word.len() > 1 {
            if word.chars().any(|c| !c.is_ascii() || matches!(c, '|' | '&' | ';' | '$' | '`')) {
                return SecurityResult::Deny(format!("obfuscated flag: {}", word));
            }
        }
    }
    SecurityResult::Passthrough
}

fn validate_brace_expansion(ctx: &ValidationContext) -> SecurityResult {
    let content = &ctx.fully_unquoted_pre_strip;
    // {a,b} pattern
    let mut depth = 0i32;
    let mut has_comma = false;
    for ch in content.chars() {
        match ch {
            '{' => { depth += 1; has_comma = false; }
            '}' if depth > 0 => {
                if has_comma { return SecurityResult::Deny("brace expansion {a,b}".into()); }
                depth -= 1;
            }
            ',' if depth > 0 => { has_comma = true; }
            _ => {}
        }
    }
    // {1..10} pattern
    if content.contains("..") {
        let bytes = content.as_bytes();
        for i in 0..bytes.len().saturating_sub(3) {
            if bytes[i] == b'{' {
                if let Some(end) = content[i..].find('}') {
                    let inner = &content[i + 1..i + end];
                    if inner.contains("..") && inner.chars().all(|c| c.is_ascii_digit() || c == '.') {
                        return SecurityResult::Deny("brace range expansion {N..M}".into());
                    }
                }
            }
        }
    }
    SecurityResult::Passthrough
}

fn validate_unicode_whitespace(ctx: &ValidationContext) -> SecurityResult {
    for ch in ctx.original.chars() {
        if matches!(ch,
            '\u{00A0}' | '\u{1680}' | '\u{2000}'..='\u{200F}' |
            '\u{2028}' | '\u{2029}' | '\u{202F}' | '\u{205F}' |
            '\u{3000}' | '\u{FEFF}'
        ) {
            return SecurityResult::Deny(format!("non-ASCII whitespace U+{:04X}", ch as u32));
        }
    }
    SecurityResult::Passthrough
}

fn validate_carriage_return(ctx: &ValidationContext) -> SecurityResult {
    if ctx.original.contains('\r') {
        SecurityResult::Deny("contains carriage return".into())
    } else {
        SecurityResult::Passthrough
    }
}

fn validate_proc_environ_access(ctx: &ValidationContext) -> SecurityResult {
    let content = &ctx.fully_unquoted;
    for path in &["/proc/", "/proc\\"] {
        if content.contains(path) {
            for sensitive in &["environ", "cmdline", "maps", "mem", "fd/"] {
                if content.contains(sensitive) {
                    return SecurityResult::Deny(format!("proc filesystem access: {}", sensitive));
                }
            }
        }
    }
    SecurityResult::Passthrough
}

fn validate_ifs_injection(ctx: &ValidationContext) -> SecurityResult {
    let content = &ctx.fully_unquoted;
    if content.contains("IFS=") || content.contains("IFS ") && content.contains('=') {
        return SecurityResult::Deny("IFS manipulation".into());
    }
    SecurityResult::Passthrough
}

fn validate_backslash_escaped_operators(ctx: &ValidationContext) -> SecurityResult {
    let content = &ctx.fully_unquoted;
    for op in &["\\|", "\\&", "\\;"] {
        if content.contains(op) {
            return SecurityResult::Deny(format!("backslash-escaped operator: {}", op));
        }
    }
    SecurityResult::Passthrough
}

fn validate_comment_quote_desync(ctx: &ValidationContext) -> SecurityResult {
    let content = &ctx.unquoted_keep_quotes;
    // Check for # immediately after a closing quote: 'x'# or "x"#
    let bytes = content.as_bytes();
    for i in 1..bytes.len() {
        if bytes[i] == b'#' && (bytes[i - 1] == b'\'' || bytes[i - 1] == b'"') {
            return SecurityResult::Deny("# adjacent to closing quote (parser confusion)".into());
        }
    }
    SecurityResult::Passthrough
}

fn validate_mid_word_hash(ctx: &ValidationContext) -> SecurityResult {
    let content = &ctx.unquoted_keep_quotes;
    let bytes = content.as_bytes();
    for i in 1..bytes.len() {
        if bytes[i] == b'#'
            && !bytes[i - 1].is_ascii_whitespace()
            && bytes[i - 1] != b'\''
            && bytes[i - 1] != b'"'
        {
            // Allow ${#var} and $# patterns
            if i >= 2 && bytes[i - 1] == b'{' && bytes[i - 2] == b'$' {
                continue;
            }
            if i >= 1 && bytes[i - 1] == b'$' {
                continue;
            }
            return SecurityResult::Deny("mid-word # (parser confusion)".into());
        }
    }
    SecurityResult::Passthrough
}

fn validate_malformed_token_injection(ctx: &ValidationContext) -> SecurityResult {
    let content = &ctx.original;
    // Check for ANSI-C quoting $'\xNN' or $'\uNNNN' which can encode arbitrary chars
    if content.contains("$'\\x") || content.contains("$'\\u") || content.contains("$'\\U") {
        return SecurityResult::Deny("ANSI-C quoting with hex/unicode escape".into());
    }
    SecurityResult::Passthrough
}

fn validate_jq_system_function(ctx: &ValidationContext) -> SecurityResult {
    if ctx.base_command != "jq" {
        return SecurityResult::Passthrough;
    }
    let content = &ctx.original;
    for func in &["system", "@sh"] {
        if content.contains(func) {
            return SecurityResult::Deny(format!("jq {} function (code execution)", func));
        }
    }
    SecurityResult::Passthrough
}

fn validate_git_commit_substitution(ctx: &ValidationContext) -> SecurityResult {
    if ctx.base_command != "git" {
        return SecurityResult::Passthrough;
    }
    let content = &ctx.original;
    if !content.contains("commit") || !content.contains("-m") {
        return SecurityResult::Passthrough;
    }
    // Check the message argument for substitution
    let unquoted = &ctx.unquoted;
    if unquoted.contains("$(") || has_unescaped_char(unquoted, '`') {
        return SecurityResult::Deny("git commit message contains command substitution".into());
    }
    SecurityResult::Passthrough
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn has_unescaped_char(content: &str, target: char) -> bool {
    let mut i = 0;
    let bytes = content.as_bytes();
    let target_byte = target as u8;
    while i < bytes.len() {
        if bytes[i] == b'\\' && i + 1 < bytes.len() {
            i += 2; // skip escaped char
            continue;
        }
        if bytes[i] == target_byte {
            return true;
        }
        i += 1;
    }
    false
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_command_allowed() {
        assert!(matches!(validate_bash_command(""), SecurityResult::Allow(_)));
        assert!(matches!(validate_bash_command("  "), SecurityResult::Allow(_)));
    }

    #[test]
    fn safe_commands_pass_through() {
        assert!(matches!(validate_bash_command("ls -la"), SecurityResult::Passthrough));
        assert!(matches!(validate_bash_command("cat foo.txt"), SecurityResult::Passthrough));
        assert!(matches!(validate_bash_command("git status"), SecurityResult::Passthrough));
    }

    #[test]
    fn command_substitution_denied() {
        assert!(matches!(validate_bash_command("echo $(whoami)"), SecurityResult::Deny(_)));
        assert!(matches!(validate_bash_command("echo `whoami`"), SecurityResult::Deny(_)));
    }

    #[test]
    fn dangerous_variables_denied() {
        assert!(matches!(validate_bash_command("IFS=: read a b"), SecurityResult::Deny(_)));
        assert!(matches!(validate_bash_command("PATH=/tmp:$PATH cmd"), SecurityResult::Deny(_)));
        assert!(matches!(validate_bash_command("LD_PRELOAD=/tmp/evil.so cmd"), SecurityResult::Deny(_)));
    }

    #[test]
    fn newlines_denied() {
        assert!(matches!(validate_bash_command("echo hello\nrm -rf /"), SecurityResult::Deny(_)));
    }

    #[test]
    fn carriage_return_denied() {
        assert!(matches!(validate_bash_command("echo hello\rmalicious"), SecurityResult::Deny(_)));
    }

    #[test]
    fn unicode_whitespace_denied() {
        assert!(matches!(validate_bash_command("echo\u{00A0}hello"), SecurityResult::Deny(_)));
    }

    #[test]
    fn zsh_dangerous_denied() {
        assert!(matches!(validate_bash_command("zmodload zsh/system"), SecurityResult::Deny(_)));
        assert!(matches!(validate_bash_command("zpty cmd ls"), SecurityResult::Deny(_)));
    }

    #[test]
    fn brace_expansion_denied() {
        assert!(matches!(validate_bash_command("echo {a,b,c}"), SecurityResult::Deny(_)));
        assert!(matches!(validate_bash_command("echo {1..10}"), SecurityResult::Deny(_)));
    }

    #[test]
    fn proc_environ_denied() {
        assert!(matches!(validate_bash_command("cat /proc/self/environ"), SecurityResult::Deny(_)));
    }

    #[test]
    fn pipe_asks() {
        assert!(matches!(validate_bash_command("ls | grep foo"), SecurityResult::Ask(_)));
    }

    #[test]
    fn semicolon_asks() {
        assert!(matches!(validate_bash_command("echo a; echo b"), SecurityResult::Ask(_)));
    }

    #[test]
    fn quoted_content_safe() {
        // Command substitution inside quotes should be stripped
        assert!(matches!(validate_bash_command("echo 'hello world'"), SecurityResult::Passthrough));
    }

    #[test]
    fn jq_system_denied() {
        assert!(matches!(validate_bash_command("jq '.[] | system(\"ls\")'"), SecurityResult::Deny(_)));
    }

    #[test]
    fn ansi_c_quoting_denied() {
        assert!(matches!(validate_bash_command("echo $'\\x41'"), SecurityResult::Deny(_)));
    }

    #[test]
    fn comment_quote_desync_denied() {
        assert!(matches!(validate_bash_command("echo 'x'#comment"), SecurityResult::Deny(_)));
    }

    #[test]
    fn incomplete_commands_denied() {
        assert!(matches!(validate_bash_command("\t-flag"), SecurityResult::Deny(_)));
        assert!(matches!(validate_bash_command("-flag"), SecurityResult::Deny(_)));
        assert!(matches!(validate_bash_command("&& echo hi"), SecurityResult::Deny(_)));
    }

    #[test]
    fn backslash_escaped_operators_denied() {
        assert!(matches!(validate_bash_command("echo \\| cat"), SecurityResult::Deny(_)));
    }

    #[test]
    fn git_commit_substitution_denied() {
        assert!(matches!(
            validate_bash_command("git commit -m \"$(whoami)\""),
            SecurityResult::Deny(_)
        ));
    }
}
