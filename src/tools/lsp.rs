//! LSP tool — semantic code intelligence via Language Server Protocol.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use serde_json::{Value, json};
use tokio::sync::OnceCell;

use crate::lsp::manager::{LspManager, load_lsp_config};

const MAX_FILE_SIZE: u64 = 10 * 1024 * 1024;

static LSP_MANAGER: OnceCell<Arc<LspManager>> = OnceCell::const_new();

/// Get or initialize the global LSP manager.
pub fn get_manager() -> Option<&'static Arc<LspManager>> {
    LSP_MANAGER.get()
}

/// Initialize the LSP manager (called once at startup if config exists).
pub fn init_manager() -> bool {
    let config = match load_lsp_config() {
        Some(c) if !c.servers.is_empty() => c,
        _ => return false,
    };
    let root = std::env::current_dir().unwrap_or_default();
    let manager = Arc::new(LspManager::new(config, root));
    LSP_MANAGER.set(manager).is_ok()
}

/// Shutdown all LSP servers (called on exit).
pub async fn shutdown() {
    if let Some(mgr) = LSP_MANAGER.get() {
        mgr.shutdown_all().await;
    }
}

pub async fn lsp_tool_response(args: &Value) -> Value {
    let manager = match LSP_MANAGER.get() {
        Some(m) => m,
        None => return error("not_initialized", "LSP not initialized. Run `zavora lsp init` first."),
    };

    let operation = match args.get("operation").and_then(Value::as_str) {
        Some(op) => op,
        None => return error("invalid_args", "'operation' is required"),
    };
    let file_path = match args.get("filePath").and_then(Value::as_str).map(str::trim) {
        Some(p) if !p.is_empty() => p,
        _ => return error("invalid_args", "'filePath' is required"),
    };
    let line = args.get("line").and_then(Value::as_u64).unwrap_or(1) as u32;
    let character = args.get("character").and_then(Value::as_u64).unwrap_or(1) as u32;

    // Resolve path
    let abs_path = if Path::new(file_path).is_absolute() {
        PathBuf::from(file_path)
    } else {
        std::env::current_dir().unwrap_or_default().join(file_path)
    };

    if !abs_path.exists() {
        return error("invalid_path", format!("file does not exist: {}", file_path));
    }
    if let Ok(meta) = std::fs::metadata(&abs_path) {
        if meta.len() > MAX_FILE_SIZE {
            return error("file_too_large", "file exceeds 10MB limit");
        }
    }

    let uri = format!("file://{}", abs_path.display());
    // LSP uses 0-based positions
    let pos = json!({ "line": line.saturating_sub(1), "character": character.saturating_sub(1) });
    let text_doc = json!({ "uri": uri });
    let text_doc_pos = json!({ "textDocument": text_doc, "position": pos });

    let cwd = std::env::current_dir().unwrap_or_default();

    let result = match operation {
        "goToDefinition" => {
            let r = manager.request(&abs_path, "textDocument/definition", text_doc_pos).await;
            format_locations(r, "definition", &cwd)
        }
        "findReferences" => {
            let params = json!({
                "textDocument": text_doc,
                "position": pos,
                "context": { "includeDeclaration": true }
            });
            let r = manager.request(&abs_path, "textDocument/references", params).await;
            format_locations(r, "reference", &cwd)
        }
        "hover" => {
            let r = manager.request(&abs_path, "textDocument/hover", text_doc_pos).await;
            format_hover(r)
        }
        "documentSymbol" => {
            let params = json!({ "textDocument": text_doc });
            let r = manager.request(&abs_path, "textDocument/documentSymbol", params).await;
            format_symbols(r, &cwd)
        }
        "workspaceSymbol" => {
            let query = args.get("query").and_then(Value::as_str).unwrap_or("");
            let params = json!({ "query": query });
            let r = manager.request(&abs_path, "workspace/symbol", params).await;
            format_symbols(r, &cwd)
        }
        "goToImplementation" => {
            let r = manager.request(&abs_path, "textDocument/implementation", text_doc_pos).await;
            format_locations(r, "implementation", &cwd)
        }
        "prepareCallHierarchy" => {
            let r = manager.request(&abs_path, "textDocument/prepareCallHierarchy", text_doc_pos).await;
            format_call_hierarchy_items(r, &cwd)
        }
        "incomingCalls" | "outgoingCalls" => {
            // Two-step: first prepare, then get calls
            let prep = manager.request(&abs_path, "textDocument/prepareCallHierarchy", text_doc_pos).await;
            match prep {
                Ok(items) => {
                    let items_arr = items.as_array().cloned().unwrap_or_default();
                    if items_arr.is_empty() {
                        Ok(json!({ "operation": operation, "result": "No call hierarchy item at this position", "filePath": file_path, "resultCount": 0 }))
                    } else {
                        let method = if operation == "incomingCalls" {
                            "callHierarchy/incomingCalls"
                        } else {
                            "callHierarchy/outgoingCalls"
                        };
                        let params = json!({ "item": items_arr[0] });
                        let r = manager.request(&abs_path, method, params).await;
                        format_calls(r, operation, &cwd)
                    }
                }
                Err(e) => Err(e),
            }
        }
        _ => {
            return error("invalid_args", format!("unknown operation: {}", operation));
        }
    };

    match result {
        Ok(v) => v,
        Err(e) => error("lsp_error", format!("{:#}", e)),
    }
}

// ---------------------------------------------------------------------------
// Formatters
// ---------------------------------------------------------------------------

fn format_locations(result: Result<Value, anyhow::Error>, kind: &str, cwd: &Path) -> Result<Value, anyhow::Error> {
    let value = result?;
    let locations = extract_locations(&value);
    if locations.is_empty() {
        return Ok(json!({ "operation": kind, "result": format!("No {}s found", kind), "resultCount": 0 }));
    }
    let mut lines = Vec::new();
    let mut files = std::collections::HashSet::new();
    for loc in &locations {
        let path = uri_to_relative(&loc.uri, cwd);
        files.insert(path.clone());
        lines.push(format!("  {}:{}:{}", path, loc.line + 1, loc.character + 1));
    }
    let header = format!("Found {} {}(s) in {} file(s):", locations.len(), kind, files.len());
    let text = format!("{}\n{}", header, lines.join("\n"));
    Ok(json!({ "operation": kind, "result": text, "resultCount": locations.len(), "fileCount": files.len() }))
}

fn format_hover(result: Result<Value, anyhow::Error>) -> Result<Value, anyhow::Error> {
    let value = result?;
    let text = if let Some(contents) = value.get("contents") {
        if let Some(s) = contents.as_str() {
            s.to_string()
        } else if let Some(obj) = contents.as_object() {
            obj.get("value").and_then(Value::as_str).unwrap_or("").to_string()
        } else if let Some(arr) = contents.as_array() {
            arr.iter().filter_map(|v| v.as_str().or_else(|| v.get("value").and_then(Value::as_str))).collect::<Vec<_>>().join("\n")
        } else {
            "No hover info".to_string()
        }
    } else {
        "No hover info".to_string()
    };
    Ok(json!({ "operation": "hover", "result": text }))
}

fn format_symbols(result: Result<Value, anyhow::Error>, cwd: &Path) -> Result<Value, anyhow::Error> {
    let value = result?;
    let arr = value.as_array().cloned().unwrap_or_default();
    if arr.is_empty() {
        return Ok(json!({ "operation": "documentSymbol", "result": "No symbols found", "resultCount": 0 }));
    }
    let mut lines = Vec::new();
    fn collect_symbols(items: &[Value], lines: &mut Vec<String>, indent: usize, cwd: &Path) {
        for item in items {
            let name = item.get("name").and_then(Value::as_str).unwrap_or("?");
            let kind = item.get("kind").and_then(Value::as_u64).map(symbol_kind_name).unwrap_or("?");
            let prefix = "  ".repeat(indent);
            if let Some(loc) = item.get("location") {
                let uri = loc.get("uri").and_then(Value::as_str).unwrap_or("");
                let line = loc.get("range").and_then(|r| r.get("start")).and_then(|s| s.get("line")).and_then(Value::as_u64).unwrap_or(0);
                let path = uri_to_relative(uri, cwd);
                lines.push(format!("{}{} {} ({}:{})", prefix, kind, name, path, line + 1));
            } else if let Some(range) = item.get("range") {
                let line = range.get("start").and_then(|s| s.get("line")).and_then(Value::as_u64).unwrap_or(0);
                lines.push(format!("{}{} {} (line {})", prefix, kind, name, line + 1));
            } else {
                lines.push(format!("{}{} {}", prefix, kind, name));
            }
            if let Some(children) = item.get("children").and_then(Value::as_array) {
                collect_symbols(children, lines, indent + 1, cwd);
            }
        }
    }
    collect_symbols(&arr, &mut lines, 0, cwd);
    Ok(json!({ "operation": "documentSymbol", "result": lines.join("\n"), "resultCount": arr.len() }))
}

fn format_call_hierarchy_items(result: Result<Value, anyhow::Error>, cwd: &Path) -> Result<Value, anyhow::Error> {
    let value = result?;
    let arr = value.as_array().cloned().unwrap_or_default();
    if arr.is_empty() {
        return Ok(json!({ "operation": "prepareCallHierarchy", "result": "No call hierarchy item at this position", "resultCount": 0 }));
    }
    let mut lines = Vec::new();
    for item in &arr {
        let name = item.get("name").and_then(Value::as_str).unwrap_or("?");
        let kind = item.get("kind").and_then(Value::as_u64).map(symbol_kind_name).unwrap_or("?");
        let uri = item.get("uri").and_then(Value::as_str).unwrap_or("");
        let line = item.get("range").and_then(|r| r.get("start")).and_then(|s| s.get("line")).and_then(Value::as_u64).unwrap_or(0);
        let path = uri_to_relative(uri, cwd);
        lines.push(format!("{} {} ({}:{})", kind, name, path, line + 1));
    }
    Ok(json!({ "operation": "prepareCallHierarchy", "result": lines.join("\n"), "resultCount": arr.len() }))
}

fn format_calls(result: Result<Value, anyhow::Error>, operation: &str, cwd: &Path) -> Result<Value, anyhow::Error> {
    let value = result?;
    let arr = value.as_array().cloned().unwrap_or_default();
    if arr.is_empty() {
        return Ok(json!({ "operation": operation, "result": format!("No {} found", operation), "resultCount": 0 }));
    }
    let key = if operation == "incomingCalls" { "from" } else { "to" };
    let mut lines = Vec::new();
    let mut files = std::collections::HashSet::new();
    for call in &arr {
        if let Some(item) = call.get(key) {
            let name = item.get("name").and_then(Value::as_str).unwrap_or("?");
            let uri = item.get("uri").and_then(Value::as_str).unwrap_or("");
            let line = item.get("range").and_then(|r| r.get("start")).and_then(|s| s.get("line")).and_then(Value::as_u64).unwrap_or(0);
            let path = uri_to_relative(uri, cwd);
            files.insert(path.clone());
            lines.push(format!("  {} ({}:{})", name, path, line + 1));
        }
    }
    let header = format!("Found {} {}:", arr.len(), operation);
    Ok(json!({ "operation": operation, "result": format!("{}\n{}", header, lines.join("\n")), "resultCount": arr.len(), "fileCount": files.len() }))
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

struct Location {
    uri: String,
    line: u64,
    character: u64,
}

fn extract_locations(value: &Value) -> Vec<Location> {
    let mut locs = Vec::new();
    if let Some(arr) = value.as_array() {
        for item in arr {
            if let Some(loc) = parse_location(item) {
                locs.push(loc);
            }
        }
    } else if let Some(loc) = parse_location(value) {
        locs.push(loc);
    }
    locs
}

fn parse_location(value: &Value) -> Option<Location> {
    // Location: { uri, range: { start: { line, character } } }
    let uri = value.get("uri").or_else(|| value.get("targetUri")).and_then(Value::as_str)?;
    let range = value.get("range").or_else(|| value.get("targetRange"))?;
    let start = range.get("start")?;
    Some(Location {
        uri: uri.to_string(),
        line: start.get("line").and_then(Value::as_u64).unwrap_or(0),
        character: start.get("character").and_then(Value::as_u64).unwrap_or(0),
    })
}

fn uri_to_relative(uri: &str, cwd: &Path) -> String {
    let path = uri.strip_prefix("file://").unwrap_or(uri);
    Path::new(path)
        .strip_prefix(cwd)
        .map(|p| p.display().to_string())
        .unwrap_or_else(|_| path.to_string())
}

fn symbol_kind_name(kind: u64) -> &'static str {
    match kind {
        1 => "File", 2 => "Module", 3 => "Namespace", 4 => "Package",
        5 => "Class", 6 => "Method", 7 => "Property", 8 => "Field",
        9 => "Constructor", 10 => "Enum", 11 => "Interface", 12 => "Function",
        13 => "Variable", 14 => "Constant", 15 => "String", 16 => "Number",
        17 => "Boolean", 18 => "Array", 19 => "Object", 20 => "Key",
        21 => "Null", 22 => "EnumMember", 23 => "Struct", 24 => "Event",
        25 => "Operator", 26 => "TypeParameter",
        _ => "Symbol",
    }
}

fn error(code: &str, message: impl Into<String>) -> Value {
    json!({ "status": "error", "code": code, "error": message.into() })
}
