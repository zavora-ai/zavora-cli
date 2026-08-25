//! Scope-aware project instruction discovery for AGENTS, Gemini, and Claude files.

use std::path::{Path, PathBuf};

use anyhow::{Context, Result};

use super::{directory_chain, home_dir, project_root};

const MAX_INSTRUCTION_FILE_BYTES: u64 = 64 * 1024;
const MAX_INSTRUCTION_TOTAL_BYTES: usize = 256 * 1024;
const MAX_IMPORT_DEPTH: usize = 5;

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ResolvedInstructions {
    pub sources: Vec<PathBuf>,
    pub deferred_sources: Vec<PathBuf>,
    pub warnings: Vec<String>,
    pub content: String,
}

fn agents_instruction_file(directory: &Path) -> Option<PathBuf> {
    ["AGENTS.override.md", "AGENTS.md"]
        .into_iter()
        .map(|name| directory.join(name))
        .find(|path| {
            path.is_file()
                && std::fs::read_to_string(path).is_ok_and(|content| !content.trim().is_empty())
        })
}

fn non_empty_file(path: &Path) -> bool {
    path.is_file() && std::fs::metadata(path).is_ok_and(|metadata| metadata.len() > 0)
}

fn read_gemini_context_names(root: &Path, home: Option<&Path>) -> Vec<String> {
    let mut names = vec!["GEMINI.md".to_string()];
    let settings = home
        .map(|home| home.join(".gemini/settings.json"))
        .into_iter()
        .chain([root.join(".gemini/settings.json")]);
    for path in settings {
        let Ok(content) = std::fs::read_to_string(path) else {
            continue;
        };
        let Ok(value) = serde_json::from_str::<serde_json::Value>(&content) else {
            continue;
        };
        let Some(file_name) = value.pointer("/context/fileName") else {
            continue;
        };
        let configured = match file_name {
            serde_json::Value::String(name) => vec![name.clone()],
            serde_json::Value::Array(values) => values
                .iter()
                .filter_map(|value| value.as_str().map(str::to_string))
                .collect(),
            _ => Vec::new(),
        };
        let valid = configured
            .into_iter()
            .filter(|name| {
                let path = Path::new(name);
                !name.trim().is_empty()
                    && path.is_relative()
                    && !path
                        .components()
                        .any(|component| matches!(component, std::path::Component::ParentDir))
            })
            .collect::<Vec<_>>();
        if !valid.is_empty() {
            names = valid;
        }
    }
    names
}

fn claude_rule_files(base: &Path) -> Vec<PathBuf> {
    if !base.is_dir() {
        return Vec::new();
    }
    let mut files = ignore::WalkBuilder::new(base)
        .hidden(false)
        .build()
        .filter_map(Result::ok)
        .filter(|entry| entry.file_type().is_some_and(|kind| kind.is_file()))
        .map(|entry| entry.into_path())
        .filter(|path| {
            path.extension()
                .and_then(|extension| extension.to_str())
                .is_some_and(|extension| extension.eq_ignore_ascii_case("md"))
        })
        .collect::<Vec<_>>();
    files.sort();
    files
}

fn has_path_scope(content: &str) -> bool {
    let Some(frontmatter) = content.strip_prefix("---") else {
        return false;
    };
    let Some((frontmatter, _)) = frontmatter.split_once("\n---") else {
        return false;
    };
    frontmatter.lines().any(|line| {
        let line = line.trim_start();
        line == "paths:" || line.starts_with("paths: [") || line.starts_with("paths:\n")
    })
}

fn push_candidate(candidates: &mut Vec<PathBuf>, path: PathBuf) {
    if non_empty_file(&path) {
        candidates.push(path);
    }
}

fn instruction_candidates(
    directory: &Path,
    gemini_names: &[String],
    deferred: &mut Vec<PathBuf>,
) -> Vec<PathBuf> {
    let mut candidates = Vec::new();
    for name in gemini_names {
        push_candidate(&mut candidates, directory.join(name));
    }
    push_candidate(&mut candidates, directory.join(".claude/CLAUDE.md"));
    push_candidate(&mut candidates, directory.join("CLAUDE.md"));
    for path in claude_rule_files(&directory.join(".claude/rules")) {
        match std::fs::read_to_string(&path) {
            Ok(content) if has_path_scope(&content) => deferred.push(path),
            Ok(_) => candidates.push(path),
            Err(_) => candidates.push(path),
        }
    }
    push_candidate(&mut candidates, directory.join("CLAUDE.local.md"));
    if let Some(path) = agents_instruction_file(directory) {
        candidates.push(path);
    }
    candidates
}

fn resolve_import_path(
    token: &str,
    source: &Path,
    home: Option<&Path>,
    allowed_roots: &[PathBuf],
) -> std::result::Result<Option<PathBuf>, String> {
    let token = token
        .trim_matches(|character: char| matches!(character, ',' | ';' | ')' | ']' | '}' | '`'));
    if token.is_empty() || token.contains("://") {
        return Ok(None);
    }
    let path = if token == "~" {
        let Some(home) = home else {
            return Ok(None);
        };
        home.to_path_buf()
    } else if let Some(relative) = token.strip_prefix("~/") {
        let Some(home) = home else {
            return Ok(None);
        };
        home.join(relative)
    } else {
        let path = Path::new(token);
        if path.is_absolute() {
            path.to_path_buf()
        } else {
            source.parent().unwrap_or_else(|| Path::new(".")).join(path)
        }
    };
    if !path.is_file() {
        return Ok(None);
    }
    let canonical = path.canonicalize().map_err(|_| {
        format!(
            "could not resolve imported instruction '{}'",
            path.display()
        )
    })?;
    if !allowed_roots.iter().any(|root| canonical.starts_with(root)) {
        return Err(format!(
            "blocked instruction import '{}' from '{}': outside trusted instruction roots",
            canonical.display(),
            source.display()
        ));
    }
    Ok(Some(canonical))
}

fn read_instruction_file(path: &Path) -> Result<String> {
    let file = std::fs::File::open(path)
        .with_context(|| format!("failed to read instructions '{}'", path.display()))?;
    let mut bytes = Vec::new();
    use std::io::Read;
    file.take(MAX_INSTRUCTION_FILE_BYTES + 1)
        .read_to_end(&mut bytes)?;
    if bytes.len() as u64 > MAX_INSTRUCTION_FILE_BYTES {
        anyhow::bail!(
            "instruction file '{}' exceeds the 64 KiB limit",
            path.display()
        );
    }
    String::from_utf8(bytes)
        .with_context(|| format!("instructions '{}' are not valid UTF-8", path.display()))
}

fn expand_instruction_file(
    path: &Path,
    home: Option<&Path>,
    allowed_roots: &[PathBuf],
    depth: usize,
    seen: &mut std::collections::BTreeSet<PathBuf>,
    sources: &mut Vec<PathBuf>,
    warnings: &mut Vec<String>,
) -> Result<String> {
    let canonical = path
        .canonicalize()
        .with_context(|| format!("failed to resolve instructions '{}'", path.display()))?;
    if !seen.insert(canonical.clone()) {
        return Ok(String::new());
    }
    sources.push(canonical.clone());
    let content = read_instruction_file(&canonical)?;
    if depth >= MAX_IMPORT_DEPTH {
        if content
            .split_whitespace()
            .any(|token| token.starts_with('@'))
        {
            warnings.push(format!(
                "import depth limit reached in '{}'",
                canonical.display()
            ));
        }
        return Ok(content);
    }

    let mut expanded = String::new();
    for line in content.lines() {
        let mut cursor = 0usize;
        let mut found = false;
        for token in line.split_whitespace() {
            let Some(import) = token.strip_prefix('@') else {
                continue;
            };
            let import_path = match resolve_import_path(import, &canonical, home, allowed_roots) {
                Ok(Some(path)) => path,
                Ok(None) => continue,
                Err(warning) => {
                    warnings.push(warning);
                    continue;
                }
            };
            let Some(offset) = line[cursor..].find(token) else {
                continue;
            };
            let start = cursor + offset;
            expanded.push_str(&line[cursor..start]);
            match expand_instruction_file(
                &import_path,
                home,
                allowed_roots,
                depth + 1,
                seen,
                sources,
                warnings,
            ) {
                Ok(imported) => expanded.push_str(&format!(
                    "\n<!-- imported: {} -->\n{}\n<!-- end import -->",
                    import_path.display(),
                    imported.trim()
                )),
                Err(error) => warnings.push(error.to_string()),
            }
            cursor = start + token.len();
            found = true;
        }
        if found {
            expanded.push_str(&line[cursor..]);
        } else {
            expanded.push_str(line);
        }
        expanded.push('\n');
    }
    Ok(expanded.trim().to_string())
}

pub fn resolve_instructions_from(cwd: &Path, home: Option<&Path>) -> Result<ResolvedInstructions> {
    let cwd = cwd
        .canonicalize()
        .with_context(|| format!("failed to resolve workspace '{}'", cwd.display()))?;
    let root = project_root(&cwd);
    let gemini_names = read_gemini_context_names(&root, home);
    let mut allowed_roots = vec![root.canonicalize().unwrap_or_else(|_| root.clone())];
    if let Some(home) = home {
        for relative in [".zavora", ".gemini", ".claude"] {
            if let Ok(canonical) = home.join(relative).canonicalize() {
                allowed_roots.push(canonical);
            }
        }
    }
    let mut candidates = Vec::new();
    let mut deferred_sources = Vec::new();

    if let Some(home) = home {
        if let Some(path) = agents_instruction_file(&home.join(".zavora")) {
            candidates.push(path);
        }
        for name in &gemini_names {
            push_candidate(&mut candidates, home.join(".gemini").join(name));
        }
        push_candidate(&mut candidates, home.join(".claude/CLAUDE.md"));
        for path in claude_rule_files(&home.join(".claude/rules")) {
            match std::fs::read_to_string(&path) {
                Ok(content) if has_path_scope(&content) => deferred_sources.push(path),
                _ => candidates.push(path),
            }
        }
    }
    for directory in directory_chain(&root, &cwd) {
        candidates.extend(instruction_candidates(
            &directory,
            &gemini_names,
            &mut deferred_sources,
        ));
    }

    let mut sources = Vec::new();
    let mut warnings = Vec::new();
    let mut seen = std::collections::BTreeSet::new();
    let mut sections = Vec::new();
    let mut total_bytes = 0usize;
    for path in candidates {
        let content = expand_instruction_file(
            &path,
            home,
            &allowed_roots,
            0,
            &mut seen,
            &mut sources,
            &mut warnings,
        )?;
        if content.trim().is_empty() {
            continue;
        }
        if total_bytes.saturating_add(content.len()) > MAX_INSTRUCTION_TOTAL_BYTES {
            warnings.push(
                "combined instructions exceed the 256 KiB limit; remaining sources were skipped"
                    .to_string(),
            );
            break;
        }
        total_bytes += content.len();
        sections.push(format!(
            "<!-- source: {} -->\n{}",
            path.display(),
            content.trim()
        ));
    }
    Ok(ResolvedInstructions {
        sources,
        deferred_sources,
        warnings,
        content: sections.join("\n\n"),
    })
}

pub fn resolve_agents_instructions_from(
    cwd: &Path,
    home: Option<&Path>,
) -> Result<ResolvedInstructions> {
    resolve_instructions_from(cwd, home)
}

pub fn resolve_workspace_instructions() -> Result<ResolvedInstructions> {
    let cwd = std::env::current_dir().context("failed to resolve current workspace")?;
    resolve_instructions_from(&cwd, home_dir().as_deref())
}

pub fn format_instructions_markdown(show_content: bool) -> String {
    match resolve_workspace_instructions() {
        Ok(resolved) => {
            let mut output = format!(
                "## Project instructions\n\n{} active source(s), {} deferred path-scoped rule(s).\n",
                resolved.sources.len(),
                resolved.deferred_sources.len()
            );
            for path in &resolved.sources {
                output.push_str(&format!("\n- active: `{}`", path.display()));
            }
            for path in &resolved.deferred_sources {
                output.push_str(&format!("\n- deferred: `{}`", path.display()));
            }
            for warning in &resolved.warnings {
                output.push_str(&format!("\n- warning: {warning}"));
            }
            if show_content && !resolved.content.is_empty() {
                output.push_str("\n\n### Resolved content\n\n");
                output.push_str(&resolved.content);
            }
            output
        }
        Err(error) => format!("## Project instructions\n\nFailed to load instructions: {error}"),
    }
}
