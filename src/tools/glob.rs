use std::path::Path;
use std::time::Instant;

use ignore::WalkBuilder;
use serde_json::{Value, json};

use super::fs_read::{enforce_workspace_path_policy, fs_read_workspace_root};

const MAX_RESULTS: usize = 100;

pub fn glob_tool_response(args: &Value) -> Value {
    let workspace_root = match fs_read_workspace_root() {
        Ok(r) => r,
        Err(e) => return error(e.code, &e.message),
    };

    let pattern = match args.get("pattern").and_then(Value::as_str).map(str::trim) {
        Some(p) if !p.is_empty() => p,
        _ => return error("invalid_args", "'pattern' is required"),
    };

    let search_root = match args.get("path").and_then(Value::as_str).map(str::trim) {
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
        &search_root.display().to_string(),
        &search_root,
        &workspace_root,
    ) {
        return error(e.code, &e.message);
    }

    let glob = match ignore::overrides::OverrideBuilder::new(&search_root)
        .add(pattern)
        .and_then(|b| b.build())
    {
        Ok(g) => g,
        Err(e) => return error("invalid_pattern", format!("bad glob pattern: {}", e)),
    };

    let start = Instant::now();
    let mut filenames = Vec::new();
    let mut truncated = false;

    let walker = WalkBuilder::new(&search_root)
        .hidden(false)
        .git_ignore(true)
        .git_global(true)
        .git_exclude(true)
        .overrides(glob)
        .build();

    for entry in walker.flatten() {
        if entry.file_type().is_some_and(|ft| ft.is_file()) {
            if filenames.len() >= MAX_RESULTS {
                truncated = true;
                break;
            }
            let path = entry.path();
            let display = path
                .strip_prefix(&workspace_root)
                .unwrap_or(path)
                .display()
                .to_string();
            filenames.push(display);
        }
    }

    filenames.sort();

    json!({
        "numFiles": filenames.len(),
        "filenames": filenames,
        "truncated": truncated,
        "durationMs": start.elapsed().as_millis() as u64,
    })
}

fn error(code: &str, message: impl Into<String>) -> Value {
    json!({ "status": "error", "code": code, "error": message.into() })
}
