use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use adk_skill::{SkillDocument, SkillIndex};
use anyhow::{Context, Result, bail};
use serde::{Deserialize, Serialize};

use crate::cli::ExtensionScope;

const MAX_INSTRUCTION_FILE_BYTES: u64 = 64 * 1024;
const MAX_INSTRUCTION_TOTAL_BYTES: usize = 256 * 1024;
const MAX_IMPORT_DEPTH: usize = 5;

const PROJECT_SKILL_DIRS: &[&str] = &[
    ".opencode/skills",
    ".gemini/skills",
    ".grok/skills",
    ".claude/skills",
    ".skills",
    ".zavora/skills",
    ".agents/skills",
];
const GLOBAL_SKILL_DIRS: &[&str] = &[
    ".config/opencode/skills",
    ".gemini/skills",
    ".grok/skills",
    ".claude/skills",
    ".skills",
    ".zavora/skills",
    ".agents/skills",
];

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ResolvedInstructions {
    pub sources: Vec<PathBuf>,
    pub deferred_sources: Vec<PathBuf>,
    pub warnings: Vec<String>,
    pub content: String,
}

fn home_dir() -> Option<PathBuf> {
    std::env::var_os("HOME").map(PathBuf::from)
}

fn project_root(cwd: &Path) -> PathBuf {
    cwd.ancestors()
        .find(|candidate| candidate.join(".git").exists())
        .unwrap_or(cwd)
        .to_path_buf()
}

fn directory_chain(root: &Path, cwd: &Path) -> Vec<PathBuf> {
    let mut chain = cwd
        .ancestors()
        .take_while(|candidate| candidate.starts_with(root))
        .map(Path::to_path_buf)
        .collect::<Vec<_>>();
    chain.reverse();
    chain
}

fn load_skill_directory(path: &Path, standard_layout: bool) -> Result<Vec<SkillDocument>> {
    if !path.is_dir() {
        return Ok(Vec::new());
    }
    // Use a deliberately absent root so adk-skill scans only the explicit
    // directory and does not fold AGENTS.md convention files into the skill index.
    let sentinel_root = path.join(".zavora-skill-discovery-sentinel");
    let index = adk_skill::load_skill_index_with_extras(&sentinel_root, &[path.to_path_buf()])
        .with_context(|| format!("failed to load skills from '{}'", path.display()))?;
    Ok(index
        .skills()
        .iter()
        .filter(|skill| {
            let file_name = skill.path.file_name().and_then(|name| name.to_str());
            if standard_layout {
                file_name.is_some_and(|name| name.eq_ignore_ascii_case("SKILL.md"))
            } else {
                !file_name.is_some_and(|name| {
                    matches!(
                        name.to_ascii_uppercase().as_str(),
                        "AGENTS.MD"
                            | "AGENT.MD"
                            | "CLAUDE.MD"
                            | "GEMINI.MD"
                            | "COPILOT.MD"
                            | "SKILLS.MD"
                            | "SOUL.MD"
                    )
                })
            }
        })
        .cloned()
        .collect())
}

pub fn load_skills_from(cwd: &Path, home: Option<&Path>) -> Result<SkillIndex> {
    let cwd = cwd
        .canonicalize()
        .with_context(|| format!("failed to resolve workspace '{}'", cwd.display()))?;
    let root = project_root(&cwd);
    let mut by_name = BTreeMap::<String, SkillDocument>::new();

    // Sources are processed from lowest to highest precedence. A project skill
    // overrides a global skill; the closest directory and the standard
    // `.agents/skills` location win within the project chain.
    if let Some(home) = home {
        for relative in GLOBAL_SKILL_DIRS {
            for skill in load_skill_directory(&home.join(relative), *relative != ".skills")? {
                by_name.insert(skill.name.clone(), skill);
            }
        }
    }
    for directory in directory_chain(&root, &cwd) {
        for relative in PROJECT_SKILL_DIRS {
            for skill in load_skill_directory(&directory.join(relative), *relative != ".skills")? {
                by_name.insert(skill.name.clone(), skill);
            }
        }
    }

    Ok(SkillIndex::new(by_name.into_values().collect()))
}

pub fn load_workspace_skills() -> Result<SkillIndex> {
    let cwd = std::env::current_dir().context("failed to resolve current workspace")?;
    let mut by_name = load_skills_from(&cwd, home_dir().as_deref())?
        .skills()
        .iter()
        .cloned()
        .map(|skill| (skill.name.clone(), skill))
        .collect::<BTreeMap<_, _>>();

    let records = all_skill_records()?;
    let disabled_roots = records
        .iter()
        .filter(|record| !record.enabled)
        .filter_map(|record| record.path.canonicalize().ok())
        .collect::<Vec<_>>();
    by_name.retain(|_, skill| {
        let canonical = skill
            .path
            .canonicalize()
            .unwrap_or_else(|_| skill.path.clone());
        !disabled_roots
            .iter()
            .any(|root| canonical.starts_with(root))
    });
    for record in records
        .iter()
        .filter(|record| record.enabled && record.linked)
    {
        for skill in load_skill_directory(&record.path, true)? {
            by_name.insert(skill.name.clone(), skill);
        }
    }

    for (plugin_name, root, standard_layout) in crate::plugins::enabled_plugin_skill_roots()? {
        for mut skill in load_skill_directory(&root, standard_layout)? {
            let original_name = skill.name.clone();
            skill.name = format!("{plugin_name}:{original_name}");
            skill.id = format!("{plugin_name}:{}", skill.id);
            skill.description = format!("{} (from plugin {plugin_name})", skill.description);
            skill.metadata.insert(
                "zavora/plugin".to_string(),
                serde_json::Value::String(plugin_name.clone()),
            );
            by_name.insert(skill.name.clone(), skill);
        }
    }

    Ok(SkillIndex::new(by_name.into_values().collect()))
}

const SKILL_STATE_VERSION: u32 = 1;

#[derive(Debug, Clone, Serialize)]
pub struct SkillRegistryEntry {
    pub name: String,
    pub category: String,
    pub repository: String,
    pub local_path: Option<PathBuf>,
}

fn registry_roots() -> Vec<PathBuf> {
    let mut roots = Vec::new();
    if let Some(configured) = std::env::var_os("ZAVORA_SKILLS_REGISTRY") {
        roots.push(PathBuf::from(configured));
    }
    if let Ok(cwd) = std::env::current_dir() {
        roots.push(cwd.join("skills-registry"));
        if let Some(parent) = cwd.parent() {
            roots.push(parent.join("skills-registry"));
        }
    }
    if let Some(home) = home_dir() {
        roots.push(home.join(".zavora/registries/skills"));
    }
    roots.dedup();
    roots
}

pub fn load_skill_registry() -> Result<Vec<SkillRegistryEntry>> {
    let Some(root) = registry_roots()
        .into_iter()
        .find(|root| root.join("registry.toml").is_file())
    else {
        return Ok(Vec::new());
    };
    let content = std::fs::read_to_string(root.join("registry.toml"))?;
    let value: toml::Value = toml::from_str(&content).context("invalid skill registry index")?;
    let local_paths = ignore::WalkBuilder::new(root.join("skills"))
        .max_depth(Some(4))
        .build()
        .filter_map(std::result::Result::ok)
        .filter(|entry| {
            entry.file_type().is_some_and(|kind| kind.is_dir())
                && entry.path().join("SKILL.md").is_file()
        })
        .filter_map(|entry| {
            let name = entry.file_name().to_str()?.to_string();
            Some((name, entry.into_path()))
        })
        .collect::<BTreeMap<_, _>>();
    let mut entries = Vec::new();
    if let Some(categories) = value.get("categories").and_then(toml::Value::as_table) {
        for (category, config) in categories {
            let Some(skills) = config.get("skills").and_then(toml::Value::as_array) else {
                continue;
            };
            for name in skills.iter().filter_map(toml::Value::as_str) {
                entries.push(SkillRegistryEntry {
                    name: name.to_string(),
                    category: category.to_string(),
                    repository: format!("https://github.com/zavora-ai/skill-{name}.git"),
                    local_path: local_paths.get(name).cloned(),
                });
            }
        }
    }
    entries.sort_by(|left, right| left.name.cmp(&right.name));
    Ok(entries)
}

pub fn search_registry(query: &str) -> Result<Vec<SkillRegistryEntry>> {
    let terms = query
        .split_whitespace()
        .map(str::to_ascii_lowercase)
        .collect::<Vec<_>>();
    Ok(load_skill_registry()?
        .into_iter()
        .filter(|entry| {
            terms.is_empty()
                || terms.iter().all(|term| {
                    entry.name.to_ascii_lowercase().contains(term)
                        || entry.category.to_ascii_lowercase().contains(term)
                })
        })
        .collect())
}

fn resolve_registry_source(source: &str) -> Result<String> {
    if Path::new(source).exists() || crate::plugins::is_git_source(source) {
        return Ok(source.to_string());
    }
    if let Some(entry) = load_skill_registry()?
        .into_iter()
        .find(|entry| entry.name == source)
    {
        return Ok(entry
            .local_path
            .map(|path| path.display().to_string())
            .unwrap_or(entry.repository));
    }
    Ok(source.to_string())
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ManagedSkillRecord {
    pub name: String,
    pub path: PathBuf,
    pub source: String,
    pub enabled: bool,
    pub linked: bool,
}

#[derive(Debug, Serialize, Deserialize)]
struct SkillState {
    #[serde(default = "skill_state_version")]
    version: u32,
    #[serde(default)]
    skills: Vec<ManagedSkillRecord>,
}

fn skill_state_version() -> u32 {
    SKILL_STATE_VERSION
}

impl Default for SkillState {
    fn default() -> Self {
        Self {
            version: SKILL_STATE_VERSION,
            skills: Vec::new(),
        }
    }
}

fn skill_paths(scope: ExtensionScope) -> Result<(PathBuf, PathBuf)> {
    match scope {
        ExtensionScope::Workspace => Ok((
            PathBuf::from(".zavora/skills.toml"),
            PathBuf::from(".zavora/skills"),
        )),
        ExtensionScope::User => {
            let home =
                home_dir().context("HOME is unavailable; user skill scope cannot be resolved")?;
            Ok((
                home.join(".zavora/skills.toml"),
                home.join(".zavora/skills"),
            ))
        }
    }
}

fn load_skill_state(path: &Path) -> Result<SkillState> {
    if !path.exists() {
        return Ok(SkillState::default());
    }
    let content = std::fs::read_to_string(path)
        .with_context(|| format!("failed to read skill state '{}'", path.display()))?;
    let state: SkillState = toml::from_str(&content)
        .with_context(|| format!("invalid skill state '{}'", path.display()))?;
    if state.version != SKILL_STATE_VERSION {
        bail!(
            "unsupported skill state version {} in '{}'",
            state.version,
            path.display()
        );
    }
    Ok(state)
}

fn save_skill_state(path: &Path, state: &SkillState) -> Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    std::fs::write(path, toml::to_string_pretty(state)?)
        .with_context(|| format!("failed to write skill state '{}'", path.display()))
}

fn all_skill_records() -> Result<Vec<ManagedSkillRecord>> {
    let mut records = Vec::new();
    for scope in [ExtensionScope::User, ExtensionScope::Workspace] {
        let (state_path, _) = skill_paths(scope)?;
        records.extend(load_skill_state(&state_path)?.skills);
    }
    Ok(records)
}

fn resolve_skill_root(path: &Path) -> Result<PathBuf> {
    let path = path
        .canonicalize()
        .with_context(|| format!("failed to resolve skill source '{}'", path.display()))?;
    let root = if path.is_file()
        && path
            .file_name()
            .and_then(|name| name.to_str())
            .is_some_and(|name| name.eq_ignore_ascii_case("SKILL.md"))
    {
        path.parent()
            .context("SKILL.md has no parent directory")?
            .to_path_buf()
    } else {
        path
    };
    if !root.join("SKILL.md").is_file() {
        bail!(
            "skill directory '{}' does not contain SKILL.md",
            root.display()
        );
    }
    Ok(root)
}

pub fn validate_skill_path(path: &Path) -> Result<SkillDocument> {
    let root = resolve_skill_root(path)?;
    let mut skills = load_skill_directory(&root, true)?;
    let canonical_manifest = root.join("SKILL.md").canonicalize()?;
    let position = skills
        .iter()
        .position(|skill| skill.path.canonicalize().ok().as_ref() == Some(&canonical_manifest))
        .context("SKILL.md did not produce a valid standard skill")?;
    let skill = skills.remove(position);
    if skill.name.len() > 64
        || skill.name.is_empty()
        || skill.name.starts_with('-')
        || skill.name.ends_with('-')
        || skill.name.contains("--")
        || !skill
            .name
            .bytes()
            .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'-')
    {
        bail!(
            "skill name '{}' must be lowercase kebab-case and at most 64 characters",
            skill.name
        );
    }
    if root.file_name().and_then(|name| name.to_str()) != Some(skill.name.as_str()) {
        bail!(
            "skill name '{}' must match directory name '{}'",
            skill.name,
            root.file_name()
                .and_then(|name| name.to_str())
                .unwrap_or_default()
        );
    }
    Ok(skill)
}

pub fn install_skill(source: &str, scope: ExtensionScope, link: bool) -> Result<SkillDocument> {
    let resolved_source = resolve_registry_source(source)?;
    let source = resolved_source.as_str();
    let (state_path, install_root) = skill_paths(scope)?;
    let mut temporary = None;
    let source_root = if crate::plugins::is_git_source(source) {
        if link {
            bail!("--link requires a local skill directory");
        }
        let temp = tempfile::Builder::new().prefix("zavora-skill-").tempdir()?;
        let destination = temp.path().join("source");
        crate::plugins::run_git(
            &[
                "clone",
                "--depth",
                "1",
                source,
                destination.to_string_lossy().as_ref(),
            ],
            "cloning skill source",
        )?;
        temporary = Some(temp);
        destination
    } else {
        resolve_skill_root(Path::new(source))?
    };
    let stored_source = if crate::plugins::is_git_source(source) {
        source.to_string()
    } else {
        source_root.display().to_string()
    };
    let inspected = validate_skill_path(&source_root)?;
    let mut state = load_skill_state(&state_path)?;
    if state
        .skills
        .iter()
        .any(|record| record.name == inspected.name)
    {
        bail!(
            "skill '{}' is already installed in {scope} scope",
            inspected.name
        );
    }
    let installed_root = if link {
        source_root
    } else {
        let target = install_root.join(&inspected.name);
        if target.exists() {
            bail!("skill target '{}' already exists", target.display());
        }
        if crate::plugins::is_git_source(source) {
            std::fs::create_dir_all(&install_root)?;
            crate::plugins::run_git(
                &[
                    "clone",
                    "--depth",
                    "1",
                    source,
                    target.to_string_lossy().as_ref(),
                ],
                "installing skill",
            )?;
        } else {
            crate::plugins::copy_tree(&source_root, &target)?;
        }
        target.canonicalize().unwrap_or(target)
    };
    drop(temporary);
    let installed = validate_skill_path(&installed_root)?;
    state.skills.push(ManagedSkillRecord {
        name: installed.name.clone(),
        path: installed_root,
        source: stored_source,
        enabled: true,
        linked: link,
    });
    state
        .skills
        .sort_by(|left, right| left.name.cmp(&right.name));
    save_skill_state(&state_path, &state)?;
    Ok(installed)
}

pub fn set_skill_enabled(name: &str, scope: ExtensionScope, enabled: bool) -> Result<()> {
    let (state_path, _) = skill_paths(scope)?;
    let mut state = load_skill_state(&state_path)?;
    let record = state
        .skills
        .iter_mut()
        .find(|record| record.name == name)
        .with_context(|| format!("skill '{name}' is not installed in {scope} scope"))?;
    record.enabled = enabled;
    save_skill_state(&state_path, &state)
}

pub fn update_skills(name: Option<&str>, scope: ExtensionScope) -> Result<Vec<String>> {
    let (state_path, install_root) = skill_paths(scope)?;
    let state = load_skill_state(&state_path)?;
    let records = state
        .skills
        .iter()
        .filter(|record| name.is_none_or(|name| record.name == name))
        .collect::<Vec<_>>();
    if records.is_empty() {
        bail!("no matching skill is installed in {scope} scope");
    }
    let mut updated = Vec::new();
    for record in records {
        if record.linked {
            validate_skill_path(&record.path)?;
            updated.push(format!("{} (linked; validated)", record.name));
        } else if record.path.join(".git").is_dir() {
            crate::plugins::run_git(
                &[
                    "-C",
                    record.path.to_string_lossy().as_ref(),
                    "pull",
                    "--ff-only",
                ],
                &format!("updating skill '{}'", record.name),
            )?;
            validate_skill_path(&record.path)?;
            updated.push(record.name.clone());
        } else {
            let source = PathBuf::from(&record.source);
            if !source.is_dir() {
                bail!(
                    "skill '{}' source '{}' is unavailable",
                    record.name,
                    source.display()
                );
            }
            let staging = install_root.join(format!(".{}.update", record.name));
            if staging.exists() {
                std::fs::remove_dir_all(&staging)?;
            }
            crate::plugins::copy_tree(&source, &staging)?;
            validate_skill_path(&staging)?;
            crate::plugins::replace_directory(&staging, &record.path)?;
            updated.push(record.name.clone());
        }
    }
    Ok(updated)
}

pub fn uninstall_skill(name: &str, scope: ExtensionScope) -> Result<bool> {
    let (state_path, install_root) = skill_paths(scope)?;
    let mut state = load_skill_state(&state_path)?;
    let index = state
        .skills
        .iter()
        .position(|record| record.name == name)
        .with_context(|| format!("skill '{name}' is not installed in {scope} scope"))?;
    let record = state.skills.remove(index);
    let install_root = install_root.canonicalize().unwrap_or_else(|_| {
        std::env::current_dir()
            .unwrap_or_default()
            .join(&install_root)
    });
    let record_path = record
        .path
        .canonicalize()
        .unwrap_or_else(|_| record.path.clone());
    let removed_files = !record.linked && record_path.starts_with(install_root);
    if removed_files && record.path.is_dir() {
        std::fs::remove_dir_all(&record.path)?;
    }
    save_skill_state(&state_path, &state)?;
    Ok(removed_files)
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

pub fn expand_skill_command(input: &str) -> Result<Option<String>> {
    let Some(command) = input.trim().strip_prefix('/') else {
        return Ok(None);
    };
    let (name, arguments) = command
        .split_once(char::is_whitespace)
        .map(|(name, arguments)| (name, arguments.trim()))
        .unwrap_or((command, ""));
    let index = load_workspace_skills()?;
    let Some(skill) = index.find_by_name(name) else {
        return Ok(None);
    };
    let request = if arguments.is_empty() {
        skill.description.as_str()
    } else {
        arguments
    };
    Ok(Some(format!(
        "{}\n\nUser request:\n{}",
        skill.engineer_prompt_block(6000),
        request
    )))
}

pub fn format_skills_markdown() -> String {
    let index = match load_workspace_skills() {
        Ok(index) => index,
        Err(error) => return format!("## Skills\n\nFailed to load skills: {error}"),
    };
    if index.is_empty() {
        return "## Skills\n\nNo skills found in `.agents/skills/`, `.zavora/skills/`, `.claude/skills/`, `.gemini/skills/`, `.grok/skills/`, `.opencode/skills/`, or enabled plugins.".to_string();
    }
    let mut output = format!("## Skills ({})\n\n", index.len());
    for skill in index.skills() {
        output.push_str(&format!(
            "- `/{}` — {}  \n  `{}`\n",
            skill.name,
            skill.description,
            skill.path.display()
        ));
    }
    output
}

#[cfg(test)]
mod tests {
    use super::*;

    fn write_skill(root: &Path, location: &str, name: &str, description: &str) {
        let directory = root.join(location).join(name);
        std::fs::create_dir_all(&directory).expect("create skill directory");
        std::fs::write(
            directory.join("SKILL.md"),
            format!("---\nname: {name}\ndescription: {description}\n---\n\n# Instructions\n\nDo the work.\n"),
        )
        .expect("write skill");
    }

    #[test]
    fn non_command_is_not_a_skill_command() {
        assert!(expand_skill_command("hello").expect("parse").is_none());
    }

    #[test]
    fn discovers_standard_agents_skills_and_excludes_agents_md() {
        let temp = tempfile::tempdir().expect("temp directory");
        std::fs::create_dir(temp.path().join(".git")).expect("git marker");
        write_skill(
            temp.path(),
            ".agents/skills",
            "spreadsheet",
            "Create spreadsheets",
        );
        std::fs::write(temp.path().join("AGENTS.md"), "Always run tests.").expect("instructions");

        let index = load_skills_from(temp.path(), None).expect("load skills");
        assert!(index.find_by_name("spreadsheet").is_some());
        assert!(index.find_by_name("agents").is_none());
    }

    #[test]
    fn standard_project_skill_overrides_legacy_and_global_skills() {
        let temp = tempfile::tempdir().expect("temp directory");
        let home = tempfile::tempdir().expect("home directory");
        std::fs::create_dir(temp.path().join(".git")).expect("git marker");
        write_skill(home.path(), ".agents/skills", "review", "Global review");
        write_skill(temp.path(), ".skills", "review", "Legacy review");
        write_skill(
            temp.path(),
            ".agents/skills",
            "review",
            "Standard project review",
        );

        let index = load_skills_from(temp.path(), Some(home.path())).expect("load skills");
        assert_eq!(
            index
                .find_by_name("review")
                .expect("review skill")
                .description,
            "Standard project review"
        );
    }

    #[test]
    fn discovers_gemini_grok_and_opencode_skill_roots() {
        let temp = tempfile::tempdir().expect("temp directory");
        std::fs::create_dir(temp.path().join(".git")).expect("git marker");
        write_skill(temp.path(), ".gemini/skills", "gemini-work", "Gemini skill");
        write_skill(temp.path(), ".grok/skills", "grok-work", "Grok skill");
        write_skill(
            temp.path(),
            ".opencode/skills",
            "opencode-work",
            "OpenCode skill",
        );

        let index = load_skills_from(temp.path(), None).expect("load skills");
        assert!(index.find_by_name("gemini-work").is_some());
        assert!(index.find_by_name("grok-work").is_some());
        assert!(index.find_by_name("opencode-work").is_some());
    }

    #[test]
    fn validates_portable_skill_name_and_directory_contract() {
        let temp = tempfile::tempdir().expect("temp directory");
        write_skill(temp.path(), "", "portable-skill", "Portable skill");
        let skill = validate_skill_path(&temp.path().join("portable-skill"))
            .expect("validate portable skill");
        assert_eq!(skill.name, "portable-skill");
    }

    #[test]
    fn resolves_agents_md_from_root_to_cwd_with_override_precedence() {
        let temp = tempfile::tempdir().expect("temp directory");
        let nested = temp.path().join("crates/app");
        std::fs::create_dir_all(&nested).expect("nested directory");
        std::fs::create_dir(temp.path().join(".git")).expect("git marker");
        std::fs::write(temp.path().join("AGENTS.md"), "Root instructions")
            .expect("root instructions");
        std::fs::write(temp.path().join("crates/AGENTS.md"), "Ignored instructions")
            .expect("intermediate instructions");
        std::fs::write(
            temp.path().join("crates/AGENTS.override.md"),
            "Override instructions",
        )
        .expect("override instructions");
        std::fs::write(nested.join("AGENTS.md"), "Nested instructions")
            .expect("nested instructions");

        let resolved = resolve_agents_instructions_from(&nested, None).expect("resolve");
        assert_eq!(resolved.sources.len(), 3);
        assert!(resolved.content.contains("Root instructions"));
        assert!(resolved.content.contains("Override instructions"));
        assert!(!resolved.content.contains("Ignored instructions"));
        assert!(resolved.content.ends_with("Nested instructions"));
    }

    #[test]
    fn resolves_gemini_claude_and_agents_with_deterministic_precedence() {
        let temp = tempfile::tempdir().expect("temp directory");
        let home = tempfile::tempdir().expect("home directory");
        let nested = temp.path().join("crates/app");
        std::fs::create_dir_all(temp.path().join(".git")).expect("git marker");
        std::fs::create_dir_all(&nested).expect("nested directory");
        std::fs::create_dir_all(home.path().join(".gemini")).expect("gemini home");
        std::fs::create_dir_all(home.path().join(".claude/rules")).expect("claude home");
        std::fs::create_dir_all(temp.path().join(".claude/rules")).expect("claude project");

        std::fs::write(home.path().join(".gemini/GEMINI.md"), "global gemini").unwrap();
        std::fs::write(home.path().join(".claude/CLAUDE.md"), "global claude").unwrap();
        std::fs::write(temp.path().join("GEMINI.md"), "root gemini").unwrap();
        std::fs::write(temp.path().join(".claude/CLAUDE.md"), "root dot claude").unwrap();
        std::fs::write(temp.path().join("CLAUDE.md"), "root claude").unwrap();
        std::fs::write(temp.path().join(".claude/rules/general.md"), "general rule").unwrap();
        std::fs::write(
            temp.path().join(".claude/rules/rust.md"),
            "---\npaths:\n  - src/**/*.rs\n---\nscoped rule",
        )
        .unwrap();
        std::fs::write(temp.path().join("CLAUDE.local.md"), "root local").unwrap();
        std::fs::write(temp.path().join("AGENTS.md"), "root agents").unwrap();
        std::fs::write(nested.join("GEMINI.md"), "nested gemini").unwrap();
        std::fs::write(nested.join("AGENTS.override.md"), "nested agents").unwrap();

        let resolved = resolve_instructions_from(&nested, Some(home.path())).unwrap();
        let positions = [
            "global gemini",
            "global claude",
            "root gemini",
            "root dot claude",
            "root claude",
            "general rule",
            "root local",
            "root agents",
            "nested gemini",
            "nested agents",
        ]
        .map(|text| resolved.content.find(text).expect("resolved content"));
        assert!(positions.windows(2).all(|pair| pair[0] < pair[1]));
        assert!(!resolved.content.contains("scoped rule"));
        assert_eq!(resolved.deferred_sources.len(), 1);
    }

    #[test]
    fn expands_instruction_imports_once_and_breaks_cycles() {
        let temp = tempfile::tempdir().expect("temp directory");
        std::fs::create_dir(temp.path().join(".git")).expect("git marker");
        std::fs::write(temp.path().join("CLAUDE.md"), "Before @./shared.md after").unwrap();
        std::fs::write(temp.path().join("shared.md"), "Shared rule. @./CLAUDE.md").unwrap();

        let resolved = resolve_instructions_from(temp.path(), None).unwrap();
        assert!(resolved.content.contains("Before"));
        assert!(resolved.content.contains("Shared rule."));
        assert_eq!(resolved.content.matches("Shared rule.").count(), 1);
        assert_eq!(resolved.sources.len(), 2);
    }

    #[test]
    fn honors_gemini_configured_context_file_names_and_deduplicates_agents() {
        let temp = tempfile::tempdir().expect("temp directory");
        std::fs::create_dir(temp.path().join(".git")).expect("git marker");
        std::fs::create_dir(temp.path().join(".gemini")).expect("settings directory");
        std::fs::write(
            temp.path().join(".gemini/settings.json"),
            r#"{"context":{"fileName":["CONTEXT.md","AGENTS.md"]}}"#,
        )
        .unwrap();
        std::fs::write(temp.path().join("GEMINI.md"), "default gemini").unwrap();
        std::fs::write(temp.path().join("CONTEXT.md"), "custom gemini").unwrap();
        std::fs::write(temp.path().join("AGENTS.md"), "native agents").unwrap();

        let resolved = resolve_instructions_from(temp.path(), None).unwrap();
        assert!(resolved.content.contains("custom gemini"));
        assert!(resolved.content.contains("native agents"));
        assert!(!resolved.content.contains("default gemini"));
        assert_eq!(resolved.content.matches("native agents").count(), 1);
        assert_eq!(resolved.sources.len(), 2);
    }

    #[test]
    fn blocks_instruction_imports_outside_trusted_roots() {
        let temp = tempfile::tempdir().expect("temp directory");
        let external = tempfile::tempdir().expect("external directory");
        std::fs::create_dir(temp.path().join(".git")).expect("git marker");
        let secret = external.path().join("private.md");
        std::fs::write(&secret, "must not reach model context").unwrap();
        std::fs::write(
            temp.path().join("CLAUDE.md"),
            format!("Import @{}", secret.display()),
        )
        .unwrap();

        let resolved = resolve_instructions_from(temp.path(), None).unwrap();
        assert!(!resolved.content.contains("must not reach model context"));
        assert!(
            resolved
                .warnings
                .iter()
                .any(|warning| warning.contains("outside trusted instruction roots"))
        );
    }
}
