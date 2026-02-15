use std::path::{Path, PathBuf};

use serde_json::{Value, json};
pub const FS_READ_DEFAULT_MAX_BYTES: usize = 8192;
pub const FS_READ_MAX_BYTES_LIMIT: usize = 65536;
pub const FS_READ_DEFAULT_MAX_LINES: usize = 200;
pub const FS_READ_MAX_LINES_LIMIT: usize = 2000;
pub const FS_READ_DEFAULT_MAX_ENTRIES: usize = 100;
pub const FS_READ_MAX_ENTRIES_LIMIT: usize = 500;
pub const FS_READ_DENIED_SEGMENTS: &[&str] = &[".git", ".zavora"];
pub const FS_READ_DENIED_FILE_NAMES: &[&str] =
    &[".env", ".env.local", ".env.development", ".env.production"];

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FsReadRequest {
    path: String,
    start_line: usize,
    max_lines: usize,
    max_bytes: usize,
    max_entries: usize,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FsReadToolError {
    pub code: &'static str,
    pub message: String,
}

impl FsReadToolError {
    fn new(code: &'static str, message: impl Into<String>) -> Self {
        Self {
            code,
            message: message.into(),
        }
    }
}

pub fn fs_read_error_payload(path: &str, err: FsReadToolError) -> Value {
    json!({
        "status": "error",
        "code": err.code,
        "error": err.message,
        "path": path
    })
}

pub fn parse_fs_read_usize_arg(
    args: &Value,
    key: &str,
    default: usize,
    min: usize,
    max: usize,
) -> Result<usize, FsReadToolError> {
    let Some(raw_value) = args.get(key) else {
        return Ok(default);
    };

    let Some(value) = raw_value.as_u64() else {
        return Err(FsReadToolError::new(
            "invalid_args",
            format!("'{key}' must be a positive integer"),
        ));
    };

    let parsed = usize::try_from(value).map_err(|_| {
        FsReadToolError::new(
            "invalid_args",
            format!("'{key}' is too large for this platform"),
        )
    })?;
    if parsed < min || parsed > max {
        return Err(FsReadToolError::new(
            "invalid_args",
            format!("'{key}' must be between {min} and {max}"),
        ));
    }

    Ok(parsed)
}

pub fn parse_fs_read_request(args: &Value) -> Result<FsReadRequest, FsReadToolError> {
    let path = args
        .get("path")
        .and_then(Value::as_str)
        .map(str::trim)
        .unwrap_or_default()
        .to_string();
    if path.is_empty() {
        return Err(FsReadToolError::new(
            "invalid_args",
            "'path' is required for fs_read",
        ));
    }

    Ok(FsReadRequest {
        path,
        start_line: parse_fs_read_usize_arg(args, "start_line", 1, 1, 1_000_000)?,
        max_lines: parse_fs_read_usize_arg(
            args,
            "max_lines",
            FS_READ_DEFAULT_MAX_LINES,
            1,
            FS_READ_MAX_LINES_LIMIT,
        )?,
        max_bytes: parse_fs_read_usize_arg(
            args,
            "max_bytes",
            FS_READ_DEFAULT_MAX_BYTES,
            1,
            FS_READ_MAX_BYTES_LIMIT,
        )?,
        max_entries: parse_fs_read_usize_arg(
            args,
            "max_entries",
            FS_READ_DEFAULT_MAX_ENTRIES,
            1,
            FS_READ_MAX_ENTRIES_LIMIT,
        )?,
    })
}

pub fn fs_read_workspace_root() -> Result<PathBuf, FsReadToolError> {
    let cwd = std::env::current_dir().map_err(|_| {
        FsReadToolError::new(
            "internal_error",
            "failed to resolve workspace root from current directory",
        )
    })?;

    cwd.canonicalize().map_err(|_| {
        FsReadToolError::new(
            "internal_error",
            "failed to canonicalize workspace root path",
        )
    })
}

pub fn resolve_fs_read_path(
    workspace_root: &Path,
    requested_path: &str,
) -> Result<PathBuf, FsReadToolError> {
    let requested = PathBuf::from(requested_path);
    let absolute = if requested.is_absolute() {
        requested
    } else {
        workspace_root.join(requested)
    };

    if !absolute.exists() {
        return Err(FsReadToolError::new(
            "invalid_path",
            format!("path '{}' does not exist", requested_path),
        ));
    }

    absolute.canonicalize().map_err(|_| {
        FsReadToolError::new(
            "invalid_path",
            format!("path '{}' could not be resolved", requested_path),
        )
    })
}

pub fn enforce_workspace_path_policy(
    requested_path: &str,
    resolved: &Path,
    workspace_root: &Path,
) -> Result<(), FsReadToolError> {
    if !resolved.starts_with(workspace_root) {
        return Err(FsReadToolError::new(
            "denied_path",
            format!(
                "fs_read denied path '{}': outside workspace root '{}'",
                requested_path,
                workspace_root.display()
            ),
        ));
    }

    for component in resolved.components() {
        let segment = component.as_os_str().to_string_lossy();
        if FS_READ_DENIED_SEGMENTS
            .iter()
            .any(|denied| segment.eq_ignore_ascii_case(denied))
        {
            return Err(FsReadToolError::new(
                "denied_path",
                format!(
                    "fs_read denied path '{}': segment '{}' is blocked by policy",
                    requested_path, segment
                ),
            ));
        }
    }

    if let Some(name) = resolved.file_name().and_then(|value| value.to_str())
        && FS_READ_DENIED_FILE_NAMES
            .iter()
            .any(|denied| name.eq_ignore_ascii_case(denied))
    {
        return Err(FsReadToolError::new(
            "denied_path",
            format!(
                "fs_read denied path '{}': filename '{}' is blocked by policy",
                requested_path, name
            ),
        ));
    }

    Ok(())
}

pub fn fs_read_display_path(path: &Path, workspace_root: &Path) -> String {
    path.strip_prefix(workspace_root)
        .map(|relative| {
            if relative.as_os_str().is_empty() {
                ".".to_string()
            } else {
                format!("./{}", relative.display())
            }
        })
        .unwrap_or_else(|_| path.display().to_string())
}

pub fn fs_read_file_payload(
    resolved: &Path,
    display_path: &str,
    request: &FsReadRequest,
) -> Result<Value, FsReadToolError> {
    let data = std::fs::read(resolved).map_err(|_| {
        FsReadToolError::new(
            "io_error",
            format!("failed to read file '{}'", display_path),
        )
    })?;

    let bytes_to_use = data.len().min(request.max_bytes);
    let truncated_by_bytes = data.len() > bytes_to_use;
    let content = String::from_utf8_lossy(&data[..bytes_to_use]).to_string();
    let lines = content.lines().collect::<Vec<&str>>();

    let start_index = request.start_line.saturating_sub(1).min(lines.len());
    let end_index = start_index
        .saturating_add(request.max_lines)
        .min(lines.len());
    let selected = lines[start_index..end_index].join("\n");
    let omitted_lines = lines.len().saturating_sub(end_index);

    Ok(json!({
        "status": "ok",
        "kind": "file",
        "path": display_path,
        "start_line": request.start_line,
        "line_count": end_index.saturating_sub(start_index),
        "omitted_lines": omitted_lines,
        "truncated": truncated_by_bytes || omitted_lines > 0,
        "content": selected
    }))
}

pub fn fs_read_directory_payload(
    resolved: &Path,
    display_path: &str,
    request: &FsReadRequest,
) -> Result<Value, FsReadToolError> {
    let mut entries = std::fs::read_dir(resolved)
        .map_err(|_| {
            FsReadToolError::new(
                "io_error",
                format!("failed to read directory '{}'", display_path),
            )
        })?
        .filter_map(|entry| entry.ok())
        .filter_map(|entry| {
            let name = entry.file_name().to_string_lossy().to_string();
            let file_type = entry.file_type().ok()?;
            let kind = if file_type.is_dir() {
                "dir"
            } else if file_type.is_file() {
                "file"
            } else if file_type.is_symlink() {
                "symlink"
            } else {
                "other"
            };
            Some((name, kind.to_string()))
        })
        .collect::<Vec<(String, String)>>();
    entries.sort_by(|a, b| a.0.cmp(&b.0));

    let total_entries = entries.len();
    let truncated = total_entries > request.max_entries;
    if truncated {
        entries.truncate(request.max_entries);
    }

    let rendered_entries = entries
        .into_iter()
        .map(|(name, kind)| json!({ "name": name, "kind": kind }))
        .collect::<Vec<Value>>();

    Ok(json!({
        "status": "ok",
        "kind": "directory",
        "path": display_path,
        "entry_count": total_entries,
        "truncated": truncated,
        "entries": rendered_entries
    }))
}

pub fn fs_read_tool_response_with_root(args: &Value, workspace_root: &Path) -> Value {
    let request = match parse_fs_read_request(args) {
        Ok(request) => request,
        Err(err) => return fs_read_error_payload("<missing>", err),
    };

    let resolved = match resolve_fs_read_path(workspace_root, &request.path) {
        Ok(path) => path,
        Err(err) => return fs_read_error_payload(&request.path, err),
    };
    if let Err(err) = enforce_workspace_path_policy(&request.path, &resolved, workspace_root) {
        return fs_read_error_payload(&request.path, err);
    }

    let display_path = fs_read_display_path(&resolved, workspace_root);
    if resolved.is_file() {
        return match fs_read_file_payload(&resolved, &display_path, &request) {
            Ok(value) => value,
            Err(err) => fs_read_error_payload(&request.path, err),
        };
    }

    if resolved.is_dir() {
        return match fs_read_directory_payload(&resolved, &display_path, &request) {
            Ok(value) => value,
            Err(err) => fs_read_error_payload(&request.path, err),
        };
    }

    fs_read_error_payload(
        &request.path,
        FsReadToolError::new(
            "unsupported_path",
            format!(
                "fs_read supports only files and directories (path '{}')",
                request.path
            ),
        ),
    )
}

pub fn fs_read_tool_response(args: &Value) -> Value {
    let workspace_root = match fs_read_workspace_root() {
        Ok(root) => root,
        Err(err) => return fs_read_error_payload("<workspace>", err),
    };
    fs_read_tool_response_with_root(args, &workspace_root)
}
