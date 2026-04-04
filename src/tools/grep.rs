use std::path::Path;
use std::process::Command;
use std::time::Instant;

use serde_json::{Value, json};

use super::fs_read::{enforce_workspace_path_policy, fs_read_workspace_root};

const DEFAULT_HEAD_LIMIT: usize = 250;
const VCS_EXCLUDES: &[&str] = &[".git", ".svn", ".hg", ".bzr", ".jj"];

pub fn grep_tool_response(args: &Value) -> Value {
    let workspace_root = match fs_read_workspace_root() {
        Ok(r) => r,
        Err(e) => return error(e.code, &e.message),
    };

    let pattern = match args.get("pattern").and_then(Value::as_str).map(str::trim) {
        Some(p) if !p.is_empty() => p.to_string(),
        _ => return error("invalid_args", "'pattern' is required"),
    };

    let search_path = match args.get("path").and_then(Value::as_str).map(str::trim) {
        Some(p) if !p.is_empty() => {
            let abs = if Path::new(p).is_absolute() {
                p.into()
            } else {
                workspace_root.join(p)
            };
            match abs.canonicalize() {
                Ok(c) => c,
                Err(_) => return error("invalid_path", format!("path '{}' does not exist", p)),
            }
        }
        _ => workspace_root.clone(),
    };

    if let Err(e) = enforce_workspace_path_policy(
        &search_path.display().to_string(),
        &search_path,
        &workspace_root,
    ) {
        return error(e.code, &e.message);
    }

    let output_mode = args
        .get("output_mode")
        .and_then(Value::as_str)
        .unwrap_or("files_with_matches");
    let head_limit = args
        .get("head_limit")
        .and_then(Value::as_u64)
        .map(|v| v as usize)
        .unwrap_or(DEFAULT_HEAD_LIMIT);
    let offset = args
        .get("offset")
        .and_then(Value::as_u64)
        .map(|v| v as usize)
        .unwrap_or(0);

    let start = Instant::now();

    // Try rg first, fall back to grep
    let result = if has_rg() {
        run_rg(&pattern, &search_path, args, output_mode)
    } else {
        run_grep_fallback(&pattern, &search_path, args, output_mode)
    };

    let raw_lines = match result {
        Ok(lines) => lines,
        Err(msg) => return error("search_error", msg),
    };

    // Apply offset + head_limit
    let total = raw_lines.len();
    let sliced: Vec<&str> = raw_lines.iter().map(|s| s.as_str()).skip(offset).collect();
    let truncated = sliced.len() > head_limit;
    let output_lines: Vec<&str> = sliced.into_iter().take(head_limit).collect();

    // Count files in results
    let num_files = count_unique_files(&output_lines, output_mode);

    json!({
        "numFiles": num_files,
        "numMatches": total,
        "results": output_lines,
        "truncated": truncated,
        "durationMs": start.elapsed().as_millis() as u64,
    })
}

fn has_rg() -> bool {
    Command::new("rg").arg("--version").output().is_ok()
}

fn run_rg(
    pattern: &str,
    search_path: &Path,
    args: &Value,
    output_mode: &str,
) -> Result<Vec<String>, String> {
    let mut cmd = Command::new("rg");

    // VCS excludes
    for dir in VCS_EXCLUDES {
        cmd.arg("--glob").arg(format!("!{}", dir));
    }

    // Output mode
    match output_mode {
        "files_with_matches" => { cmd.arg("-l"); }
        "count" => { cmd.arg("-c"); }
        _ => { cmd.arg("-n"); } // content mode: show line numbers
    }

    // Optional flags
    if args.get("-i").and_then(Value::as_bool).unwrap_or(false)
        || args.get("case_insensitive").and_then(Value::as_bool).unwrap_or(false)
    {
        cmd.arg("-i");
    }
    if let Some(g) = args.get("glob").and_then(Value::as_str) {
        cmd.arg("--glob").arg(g);
    }
    if let Some(t) = args.get("file_type").and_then(Value::as_str)
        .or_else(|| args.get("type").and_then(Value::as_str))
    {
        cmd.arg("--type").arg(t);
    }
    if args.get("multiline").and_then(Value::as_bool).unwrap_or(false) {
        cmd.arg("-U").arg("--multiline-dotall");
    }

    // Context lines (content mode only)
    if output_mode == "content" {
        if let Some(n) = args.get("-B").and_then(Value::as_u64) {
            cmd.arg("-B").arg(n.to_string());
        }
        if let Some(n) = args.get("-A").and_then(Value::as_u64) {
            cmd.arg("-A").arg(n.to_string());
        }
        let ctx = args.get("-C").and_then(Value::as_u64)
            .or_else(|| args.get("context").and_then(Value::as_u64));
        if let Some(n) = ctx {
            cmd.arg("-C").arg(n.to_string());
        }
    }

    cmd.arg("--").arg(pattern).arg(search_path);

    let output = cmd.output().map_err(|e| format!("failed to run rg: {}", e))?;

    // rg exits 1 for no matches — that's not an error
    if !output.status.success() && output.status.code() != Some(1) {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(format!("rg failed: {}", stderr.trim()));
    }

    Ok(String::from_utf8_lossy(&output.stdout)
        .lines()
        .filter(|l| !l.is_empty())
        .map(String::from)
        .collect())
}

fn run_grep_fallback(
    pattern: &str,
    search_path: &Path,
    args: &Value,
    output_mode: &str,
) -> Result<Vec<String>, String> {
    let mut cmd = Command::new("grep");
    cmd.arg("-rn"); // recursive + line numbers

    if args.get("-i").and_then(Value::as_bool).unwrap_or(false)
        || args.get("case_insensitive").and_then(Value::as_bool).unwrap_or(false)
    {
        cmd.arg("-i");
    }
    if let Some(g) = args.get("glob").and_then(Value::as_str) {
        cmd.arg("--include").arg(g);
    }

    match output_mode {
        "files_with_matches" => { cmd.arg("-l"); }
        "count" => { cmd.arg("-c"); }
        _ => {}
    }

    // Exclude VCS dirs
    for dir in VCS_EXCLUDES {
        cmd.arg("--exclude-dir").arg(*dir);
    }

    cmd.arg("-E").arg(pattern).arg(search_path);

    let output = cmd.output().map_err(|e| format!("failed to run grep: {}", e))?;

    if !output.status.success() && output.status.code() != Some(1) {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(format!("grep failed: {}", stderr.trim()));
    }

    Ok(String::from_utf8_lossy(&output.stdout)
        .lines()
        .filter(|l| !l.is_empty())
        .map(String::from)
        .collect())
}

fn count_unique_files(lines: &[&str], output_mode: &str) -> usize {
    match output_mode {
        "files_with_matches" => lines.len(),
        _ => {
            let mut files = std::collections::HashSet::new();
            for line in lines {
                if let Some(path) = line.split(':').next() {
                    files.insert(path);
                }
            }
            files.len()
        }
    }
}

fn error(code: &str, message: impl Into<String>) -> Value {
    json!({ "status": "error", "code": code, "error": message.into() })
}
