use std::path::Path;

use serde_json::{Value, json};
use similar::TextDiff;

use super::fs_read::{
    enforce_workspace_path_policy, fs_read_display_path, fs_read_workspace_root,
};

const MAX_FILE_SIZE: u64 = 10 * 1024 * 1024; // 10 MB

pub fn file_edit_tool_response(args: &Value) -> Value {
    let workspace_root = match fs_read_workspace_root() {
        Ok(root) => root,
        Err(err) => return error_payload("<workspace>", &err.code, &err.message),
    };

    let file_path = match args.get("file_path").and_then(Value::as_str).map(str::trim) {
        Some(p) if !p.is_empty() => p,
        _ => return error_payload("<missing>", "invalid_args", "'file_path' is required"),
    };
    let old_string = match args.get("old_string").and_then(Value::as_str) {
        Some(s) => s,
        None => return error_payload(file_path, "invalid_args", "'old_string' is required"),
    };
    let new_string = match args.get("new_string").and_then(Value::as_str) {
        Some(s) => s,
        None => return error_payload(file_path, "invalid_args", "'new_string' is required"),
    };
    let replace_all = args
        .get("replace_all")
        .and_then(Value::as_bool)
        .unwrap_or(false);

    if old_string == new_string {
        return error_payload(file_path, "no_change", "old_string and new_string are identical");
    }

    // Resolve path (reuse fs_read path resolution which requires the file to exist)
    let resolved = match resolve_edit_path(&workspace_root, file_path) {
        Ok(p) => p,
        Err(v) => return v,
    };

    // Size check
    if let Ok(meta) = std::fs::metadata(&resolved) {
        if meta.len() > MAX_FILE_SIZE {
            return error_payload(
                file_path,
                "file_too_large",
                format!("file exceeds 10MB limit ({}MB)", meta.len() / (1024 * 1024)),
            );
        }
    }

    // Read
    let original = match std::fs::read_to_string(&resolved) {
        Ok(c) => c,
        Err(_) => {
            return error_payload(file_path, "io_error", format!("failed to read '{}'", file_path))
        }
    };

    // Count occurrences
    let count = original.matches(old_string).count();

    if count == 0 {
        let hint = find_closest_match(&original, old_string);
        let msg = match hint {
            Some(h) => format!(
                "old_string not found in '{}'. Did you mean: \"{}\"?",
                file_path, h
            ),
            None => format!("old_string not found in '{}'", file_path),
        };
        return error_payload(file_path, "no_match", msg);
    }

    if count > 1 && !replace_all {
        return error_payload(
            file_path,
            "ambiguous_match",
            format!(
                "old_string matches {} locations in '{}'. Set replace_all=true or provide a more specific string.",
                count, file_path
            ),
        );
    }

    // Replace
    let updated = if replace_all {
        original.replace(old_string, new_string)
    } else {
        original.replacen(old_string, new_string, 1)
    };

    // Preserve line endings
    let updated = preserve_line_endings(&original, &updated);

    // Write
    if std::fs::write(&resolved, updated.as_bytes()).is_err() {
        return error_payload(file_path, "io_error", format!("failed to write '{}'", file_path));
    }

    // Diff
    let display = fs_read_display_path(&resolved, &workspace_root);
    let diff = TextDiff::from_lines(&original, &updated);
    let patch = diff
        .unified_diff()
        .header(&format!("a/{}", display), &format!("b/{}", display))
        .to_string();

    json!({
        "status": "ok",
        "path": display,
        "replacements": count,
        "diff": patch,
    })
}

/// Resolve and validate path for editing — must exist, be a file, be inside workspace.
fn resolve_edit_path(workspace_root: &Path, requested: &str) -> Result<std::path::PathBuf, Value> {
    let path = std::path::PathBuf::from(requested);
    let absolute = if path.is_absolute() {
        path
    } else {
        workspace_root.join(path)
    };

    if !absolute.exists() {
        return Err(error_payload(
            requested,
            "invalid_path",
            format!("'{}' does not exist", requested),
        ));
    }

    let resolved = absolute.canonicalize().map_err(|_| {
        error_payload(
            requested,
            "invalid_path",
            format!("'{}' could not be resolved", requested),
        )
    })?;

    if !resolved.is_file() {
        return Err(error_payload(
            requested,
            "invalid_path",
            format!("'{}' is not a file", requested),
        ));
    }

    if let Err(e) = enforce_workspace_path_policy(requested, &resolved, workspace_root) {
        return Err(error_payload(requested, e.code, &e.message));
    }

    Ok(resolved)
}

fn error_payload(path: &str, code: &str, message: impl Into<String>) -> Value {
    json!({ "status": "error", "code": code, "error": message.into(), "path": path })
}

fn preserve_line_endings(original: &str, updated: &str) -> String {
    let uses_crlf = original.as_bytes().windows(2).any(|w| w == b"\r\n");
    let uses_lf = original.contains('\n') && !uses_crlf;
    if uses_crlf && !updated.contains("\r\n") {
        updated.replace('\n', "\r\n")
    } else if uses_lf && updated.contains("\r\n") {
        updated.replace("\r\n", "\n")
    } else {
        updated.to_string()
    }
}

/// Find the most similar line-chunk in the file to the target string.
fn find_closest_match(content: &str, target: &str) -> Option<String> {
    #[cfg(feature = "semantic-search")]
    {
        let target_lines: Vec<&str> = target.lines().collect();
        let target_len = target_lines.len().max(1);
        let content_lines: Vec<&str> = content.lines().collect();
        if content_lines.is_empty() {
            return None;
        }

        let mut best_score = 0.0_f64;
        let mut best_chunk = String::new();

        for start in 0..content_lines.len() {
            let end = (start + target_len).min(content_lines.len());
            let chunk = content_lines[start..end].join("\n");
            let score = strsim::normalized_levenshtein(&chunk, target);
            if score > best_score && score > 0.5 {
                best_score = score;
                best_chunk = chunk;
            }
        }

        if best_chunk.is_empty() {
            None
        } else {
            Some(best_chunk)
        }
    }

    #[cfg(not(feature = "semantic-search"))]
    {
        // Without strsim, do a simple substring prefix search
        let prefix = &target[..target.len().min(40)];
        content
            .lines()
            .find(|line| line.contains(prefix))
            .map(|line| line.to_string())
    }
}
