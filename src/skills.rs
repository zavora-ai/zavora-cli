use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use adk_skill::{SkillDocument, SkillIndex};
use anyhow::{Context, Result, bail};
use serde::{Deserialize, Serialize};

use crate::cli::ExtensionScope;

mod instructions;

pub use instructions::{
    ResolvedInstructions, format_instructions_markdown, resolve_agents_instructions_from,
    resolve_instructions_from, resolve_workspace_instructions,
};

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

const BUNDLED_SKILLS: &[(&str, &str)] = &[
    (
        "capability-audit",
        include_str!("../.agents/skills/capability-audit/SKILL.md"),
    ),
    (
        "device-management",
        include_str!("../.agents/skills/device-management/SKILL.md"),
    ),
    ("docx", include_str!("../.agents/skills/docx/SKILL.md")),
    (
        "email-operations",
        include_str!("../.agents/skills/email-operations/SKILL.md"),
    ),
    ("pdf", include_str!("../.agents/skills/pdf/SKILL.md")),
    ("pptx", include_str!("../.agents/skills/pptx/SKILL.md")),
    (
        "repository-development",
        include_str!("../.agents/skills/repository-development/SKILL.md"),
    ),
    (
        "source-research",
        include_str!("../.agents/skills/source-research/SKILL.md"),
    ),
    ("xlsx", include_str!("../.agents/skills/xlsx/SKILL.md")),
];

fn home_dir() -> Option<PathBuf> {
    std::env::var_os("HOME").map(PathBuf::from)
}

fn bundled_skill_root() -> Option<PathBuf> {
    std::env::var_os("ZAVORA_BUNDLED_SKILLS_DIR")
        .map(PathBuf::from)
        .or_else(|| home_dir().map(|home| home.join(".zavora/bundled/skills")))
}

/// Materialize the embedded standard skills into Zavora-owned state.
///
/// Cargo cannot install auxiliary files next to a binary. Embedding makes the
/// same skills available to cargo, npm, Homebrew, and release-archive users;
/// materializing them gives ADK-Skill ordinary files for progressive loading.
/// Workspace and user skills are loaded later and retain normal precedence.
fn ensure_bundled_skills() -> Result<Option<PathBuf>> {
    let Some(root) = bundled_skill_root() else {
        return Ok(None);
    };
    for (name, content) in BUNDLED_SKILLS {
        let directory = root.join(name);
        let path = directory.join("SKILL.md");
        let current = std::fs::read_to_string(&path).ok();
        if current.as_deref() == Some(*content) {
            continue;
        }
        std::fs::create_dir_all(&directory).with_context(|| {
            format!(
                "failed to create bundled skill directory '{}'",
                directory.display()
            )
        })?;
        std::fs::write(&path, content)
            .with_context(|| format!("failed to install bundled skill '{}'", path.display()))?;
    }
    Ok(Some(root))
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
    let mut by_name = BTreeMap::new();
    if let Some(root) = ensure_bundled_skills()? {
        for skill in load_skill_directory(&root, true)? {
            by_name.insert(skill.name.clone(), skill);
        }
    }
    by_name.extend(
        load_skills_from(&cwd, home_dir().as_deref())?
            .skills()
            .iter()
            .cloned()
            .map(|skill| (skill.name.clone(), skill)),
    );

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

/// Reduce a loaded skill index using the same wildcard semantics as tool
/// policy. An empty allow list means all enabled skills; deny rules win.
pub fn filter_skill_index(
    index: SkillIndex,
    allow_patterns: &[String],
    deny_patterns: &[String],
) -> SkillIndex {
    let skills = index
        .skills()
        .iter()
        .filter(|skill| {
            let allowed = allow_patterns.is_empty()
                || allow_patterns.iter().any(|pattern| {
                    crate::tool_policy::matches_wildcard(pattern.trim(), &skill.name)
                });
            let denied = deny_patterns
                .iter()
                .any(|pattern| crate::tool_policy::matches_wildcard(pattern.trim(), &skill.name));
            allowed && !denied
        })
        .cloned()
        .collect();
    SkillIndex::new(skills)
}

pub fn load_agent_skills(
    allow_patterns: &[String],
    deny_patterns: &[String],
) -> Result<SkillIndex> {
    Ok(filter_skill_index(
        load_workspace_skills()?,
        allow_patterns,
        deny_patterns,
    ))
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
    fn agent_skill_policy_applies_allow_then_deny_wildcards() {
        let temp = tempfile::tempdir().expect("temp directory");
        std::fs::create_dir(temp.path().join(".git")).expect("git marker");
        write_skill(
            temp.path(),
            ".agents/skills",
            "repository-review",
            "Review code",
        );
        write_skill(
            temp.path(),
            ".agents/skills",
            "repository-deploy",
            "Deploy code",
        );
        write_skill(
            temp.path(),
            ".agents/skills",
            "source-research",
            "Research sources",
        );
        let index = load_skills_from(temp.path(), None).expect("load skills");
        let filtered = filter_skill_index(
            index,
            &["repository-*".to_string()],
            &["*-deploy".to_string()],
        );
        assert!(filtered.find_by_name("repository-review").is_some());
        assert!(filtered.find_by_name("repository-deploy").is_none());
        assert!(filtered.find_by_name("source-research").is_none());
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
