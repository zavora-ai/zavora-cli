use std::fs::OpenOptions;
use std::path::{Path, PathBuf};

use serde_json::{Value, json};

use super::fs_read::{
    fs_read_workspace_root, fs_read_display_path, enforce_workspace_path_policy,

};
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FsWriteMode {
    Create,
    Overwrite,
    Append,
    Patch,
}

impl FsWriteMode {
    fn label(self) -> &'static str {
        match self {
            FsWriteMode::Create => "create",
            FsWriteMode::Overwrite => "overwrite",
            FsWriteMode::Append => "append",
            FsWriteMode::Patch => "patch",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FsWritePatch {
    find: String,
    replace: String,
    replace_all: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FsWriteRequest {
    path: String,
    mode: FsWriteMode,
    content: Option<String>,
    patch: Option<FsWritePatch>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FsWriteToolError {
    code: &'static str,
    message: String,
}

impl FsWriteToolError {
    fn new(code: &'static str, message: impl Into<String>) -> Self {
        Self {
            code,
            message: message.into(),
        }
    }
}

pub fn fs_write_error_payload(path: &str, err: FsWriteToolError) -> Value {
    json!({
        "status": "error",
        "code": err.code,
        "error": err.message,
        "path": path
    })
}

pub fn parse_fs_write_mode(args: &Value) -> Result<FsWriteMode, FsWriteToolError> {
    let mode = args
        .get("mode")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .unwrap_or("overwrite")
        .to_ascii_lowercase();

    match mode.as_str() {
        "create" => Ok(FsWriteMode::Create),
        "overwrite" | "update" => Ok(FsWriteMode::Overwrite),
        "append" => Ok(FsWriteMode::Append),
        "patch" => Ok(FsWriteMode::Patch),
        _ => Err(FsWriteToolError::new(
            "invalid_args",
            "mode must be one of: create, overwrite, append, patch",
        )),
    }
}

pub fn parse_fs_write_patch(args: &Value) -> Result<Option<FsWritePatch>, FsWriteToolError> {
    let Some(raw_patch) = args.get("patch") else {
        return Ok(None);
    };

    let Some(patch_obj) = raw_patch.as_object() else {
        return Err(FsWriteToolError::new(
            "malformed_edit",
            "'patch' must be an object with 'find' and 'replace' fields",
        ));
    };

    let find = patch_obj
        .get("find")
        .and_then(Value::as_str)
        .map(str::to_string)
        .unwrap_or_default();
    let replace = patch_obj
        .get("replace")
        .and_then(Value::as_str)
        .map(str::to_string)
        .unwrap_or_default();
    let replace_all = patch_obj
        .get("replace_all")
        .and_then(Value::as_bool)
        .unwrap_or(false);

    Ok(Some(FsWritePatch {
        find,
        replace,
        replace_all,
    }))
}

pub fn parse_fs_write_request(args: &Value) -> Result<FsWriteRequest, FsWriteToolError> {
    let path = args
        .get("path")
        .and_then(Value::as_str)
        .map(str::trim)
        .unwrap_or_default()
        .to_string();
    if path.is_empty() {
        return Err(FsWriteToolError::new(
            "invalid_args",
            "'path' is required for fs_write",
        ));
    }

    let mode = parse_fs_write_mode(args)?;
    let content = args
        .get("content")
        .and_then(Value::as_str)
        .map(str::to_string);
    let patch = parse_fs_write_patch(args)?;

    match mode {
        FsWriteMode::Create | FsWriteMode::Overwrite | FsWriteMode::Append => {
            if content.is_none() {
                return Err(FsWriteToolError::new(
                    "invalid_args",
                    "'content' is required for create/overwrite/append modes",
                ));
            }
        }
        FsWriteMode::Patch => {
            let Some(patch_ref) = patch.as_ref() else {
                return Err(FsWriteToolError::new(
                    "malformed_edit",
                    "'patch' object is required for patch mode",
                ));
            };
            if patch_ref.find.is_empty() {
                return Err(FsWriteToolError::new(
                    "malformed_edit",
                    "'patch.find' cannot be empty",
                ));
            }
        }
    }

    Ok(FsWriteRequest {
        path,
        mode,
        content,
        patch,
    })
}

pub fn resolve_fs_write_path(
    workspace_root: &Path,
    requested_path: &str,
) -> Result<PathBuf, FsWriteToolError> {
    let requested = PathBuf::from(requested_path);
    let absolute = if requested.is_absolute() {
        requested
    } else {
        workspace_root.join(requested)
    };

    let mut existing = absolute.as_path();
    while !existing.exists() {
        existing = existing.parent().ok_or_else(|| {
            FsWriteToolError::new(
                "invalid_path",
                format!("path '{}' has no resolvable parent", requested_path),
            )
        })?;
    }

    let canonical_existing = existing.canonicalize().map_err(|_| {
        FsWriteToolError::new(
            "invalid_path",
            format!("path '{}' could not be resolved", requested_path),
        )
    })?;
    let suffix = absolute.strip_prefix(existing).map_err(|_| {
        FsWriteToolError::new(
            "invalid_path",
            format!("path '{}' could not be normalized", requested_path),
        )
    })?;

    if suffix.as_os_str().is_empty() {
        return Ok(canonical_existing);
    }

    Ok(canonical_existing.join(suffix))
}

pub fn fs_write_ok_payload(
    display_path: &str,
    mode: FsWriteMode,
    changed: bool,
    bytes_written: usize,
    replaced_count: usize,
) -> Value {
    json!({
        "status": "ok",
        "kind": "fs_write",
        "path": display_path,
        "mode": mode.label(),
        "changed": changed,
        "bytes_written": bytes_written,
        "replaced_count": replaced_count
    })
}

pub fn fs_write_tool_response_with_root(args: &Value, workspace_root: &Path) -> Value {
    let request = match parse_fs_write_request(args) {
        Ok(request) => request,
        Err(err) => return fs_write_error_payload("<missing>", err),
    };

    let resolved = match resolve_fs_write_path(workspace_root, &request.path) {
        Ok(path) => path,
        Err(err) => return fs_write_error_payload(&request.path, err),
    };
    if let Err(err) = enforce_workspace_path_policy(&request.path, &resolved, workspace_root) {
        return fs_write_error_payload(&request.path, FsWriteToolError::new(err.code, err.message));
    }

    let display_path = fs_read_display_path(&resolved, workspace_root);
    let result = match request.mode {
        FsWriteMode::Create => {
            if resolved.exists() {
                Err(FsWriteToolError::new(
                    "invalid_path",
                    format!("file '{}' already exists", request.path),
                ))
            } else {
                if let Some(parent) = resolved.parent()
                    && std::fs::create_dir_all(parent).is_err()
                {
                    return fs_write_error_payload(
                        &request.path,
                        FsWriteToolError::new(
                            "io_error",
                            format!("failed to create parent directories for '{}'", request.path),
                        ),
                    );
                }
                let content = request.content.as_deref().unwrap_or_default();
                std::fs::write(&resolved, content.as_bytes())
                    .map(|_| {
                        fs_write_ok_payload(&display_path, request.mode, true, content.len(), 0)
                    })
                    .map_err(|_| {
                        FsWriteToolError::new(
                            "io_error",
                            format!("failed to write '{}'", request.path),
                        )
                    })
            }
        }
        FsWriteMode::Overwrite => {
            if let Some(parent) = resolved.parent()
                && std::fs::create_dir_all(parent).is_err()
            {
                return fs_write_error_payload(
                    &request.path,
                    FsWriteToolError::new(
                        "io_error",
                        format!("failed to create parent directories for '{}'", request.path),
                    ),
                );
            }
            let content = request.content.as_deref().unwrap_or_default();
            std::fs::write(&resolved, content.as_bytes())
                .map(|_| fs_write_ok_payload(&display_path, request.mode, true, content.len(), 0))
                .map_err(|_| {
                    FsWriteToolError::new("io_error", format!("failed to write '{}'", request.path))
                })
        }
        FsWriteMode::Append => {
            if let Some(parent) = resolved.parent()
                && std::fs::create_dir_all(parent).is_err()
            {
                return fs_write_error_payload(
                    &request.path,
                    FsWriteToolError::new(
                        "io_error",
                        format!("failed to create parent directories for '{}'", request.path),
                    ),
                );
            }
            let content = request.content.as_deref().unwrap_or_default();
            OpenOptions::new()
                .append(true)
                .create(true)
                .open(&resolved)
                .and_then(|mut file| std::io::Write::write_all(&mut file, content.as_bytes()))
                .map(|_| fs_write_ok_payload(&display_path, request.mode, true, content.len(), 0))
                .map_err(|_| {
                    FsWriteToolError::new(
                        "io_error",
                        format!("failed to append to '{}'", request.path),
                    )
                })
        }
        FsWriteMode::Patch => {
            if !resolved.exists() {
                Err(FsWriteToolError::new(
                    "invalid_path",
                    format!("file '{}' does not exist for patch mode", request.path),
                ))
            } else {
                let patch = request.patch.as_ref().expect("patch mode validated");
                let original = match std::fs::read_to_string(&resolved) {
                    Ok(content) => content,
                    Err(_) => {
                        return fs_write_error_payload(
                            &request.path,
                            FsWriteToolError::new(
                                "io_error",
                                format!("failed to read '{}' for patch mode", request.path),
                            ),
                        );
                    }
                };

                let replaced_count = if patch.replace_all {
                    original.matches(&patch.find).count()
                } else if original.contains(&patch.find) {
                    1
                } else {
                    0
                };
                if replaced_count == 0 {
                    Err(FsWriteToolError::new(
                        "malformed_edit",
                        format!(
                            "patch.find value not found in '{}': '{}'",
                            request.path, patch.find
                        ),
                    ))
                } else {
                    let updated = if patch.replace_all {
                        original.replace(&patch.find, &patch.replace)
                    } else {
                        original.replacen(&patch.find, &patch.replace, 1)
                    };
                    let changed = updated != original;
                    std::fs::write(&resolved, updated.as_bytes())
                        .map(|_| {
                            fs_write_ok_payload(
                                &display_path,
                                request.mode,
                                changed,
                                updated.len(),
                                replaced_count,
                            )
                        })
                        .map_err(|_| {
                            FsWriteToolError::new(
                                "io_error",
                                format!("failed to write patched content to '{}'", request.path),
                            )
                        })
                }
            }
        }
    };

    match result {
        Ok(payload) => payload,
        Err(err) => fs_write_error_payload(&request.path, err),
    }
}

pub fn fs_write_tool_response(args: &Value) -> Value {
    let workspace_root = match fs_read_workspace_root() {
        Ok(root) => root,
        Err(err) => {
            return fs_write_error_payload(
                "<workspace>",
                FsWriteToolError::new(err.code, err.message),
            );
        }
    };
    fs_write_tool_response_with_root(args, &workspace_root)
}

