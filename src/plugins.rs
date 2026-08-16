use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::path::{Component, Path, PathBuf};
use std::process::Command;

use anyhow::{Context, Result, bail};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::cli::ExtensionScope;
use crate::config::McpServerConfig;

const STATE_VERSION: u32 = 1;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum PluginEcosystem {
    Zavora,
    Codex,
    Claude,
    Gemini,
    Grok,
    OpenCode,
}

impl std::fmt::Display for PluginEcosystem {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(match self {
            Self::Zavora => "zavora",
            Self::Codex => "codex",
            Self::Claude => "claude",
            Self::Gemini => "gemini",
            Self::Grok => "grok",
            Self::OpenCode => "opencode",
        })
    }
}

#[derive(Debug, Clone, Default, Serialize)]
pub struct PluginComponents {
    pub skill_roots: Vec<PathBuf>,
    pub agent_roots: Vec<PathBuf>,
    pub command_roots: Vec<PathBuf>,
    pub hook_files: Vec<PathBuf>,
    pub mcp_files: Vec<PathBuf>,
    pub app_files: Vec<PathBuf>,
    pub executable_entrypoints: Vec<PathBuf>,
    #[serde(skip)]
    pub inline_mcp: Option<Value>,
}

#[derive(Debug, Clone, Serialize)]
pub struct PluginDescriptor {
    pub name: String,
    pub display_name: String,
    pub version: Option<String>,
    pub description: Option<String>,
    pub ecosystem: PluginEcosystem,
    pub root: PathBuf,
    pub manifest: Option<PathBuf>,
    pub enabled: bool,
    pub managed: bool,
    pub linked: bool,
    pub source: Option<String>,
    pub components: PluginComponents,
    pub warnings: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PluginRecord {
    pub name: String,
    pub path: PathBuf,
    pub source: String,
    pub enabled: bool,
    pub linked: bool,
    pub ecosystem: PluginEcosystem,
}

#[derive(Debug, Default, Serialize, Deserialize)]
struct PluginState {
    #[serde(default = "state_version")]
    version: u32,
    #[serde(default)]
    plugins: Vec<PluginRecord>,
}

fn state_version() -> u32 {
    STATE_VERSION
}

#[derive(Debug, Clone)]
pub struct PluginPaths {
    pub state: PathBuf,
    pub install_root: PathBuf,
}

pub fn plugin_paths(scope: ExtensionScope) -> Result<PluginPaths> {
    match scope {
        ExtensionScope::Workspace => Ok(PluginPaths {
            state: PathBuf::from(".zavora/plugins.toml"),
            install_root: PathBuf::from(".zavora/plugins"),
        }),
        ExtensionScope::User => {
            let home = std::env::var_os("HOME")
                .map(PathBuf::from)
                .context("HOME is unavailable; user plugin scope cannot be resolved")?;
            Ok(PluginPaths {
                state: home.join(".zavora/plugins.toml"),
                install_root: home.join(".zavora/plugins"),
            })
        }
    }
}

fn load_state(path: &Path) -> Result<PluginState> {
    if !path.exists() {
        return Ok(PluginState {
            version: STATE_VERSION,
            plugins: Vec::new(),
        });
    }
    let content = std::fs::read_to_string(path)
        .with_context(|| format!("failed to read plugin state '{}'", path.display()))?;
    let state: PluginState = toml::from_str(&content)
        .with_context(|| format!("invalid plugin state '{}'", path.display()))?;
    if state.version != STATE_VERSION {
        bail!(
            "unsupported plugin state version {} in '{}'",
            state.version,
            path.display()
        );
    }
    Ok(state)
}

fn save_state(path: &Path, state: &PluginState) -> Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).with_context(|| {
            format!(
                "failed to create plugin state directory '{}'",
                parent.display()
            )
        })?;
    }
    let payload = toml::to_string_pretty(state).context("failed to serialize plugin state")?;
    std::fs::write(path, payload)
        .with_context(|| format!("failed to write plugin state '{}'", path.display()))
}

fn is_valid_name(name: &str) -> bool {
    !name.is_empty()
        && name.len() <= 64
        && !name.starts_with('-')
        && !name.ends_with('-')
        && !name.contains("--")
        && name
            .bytes()
            .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'-')
}

fn normalize_name(value: &str) -> String {
    let mut output = String::new();
    let mut separator = false;
    for character in value.chars() {
        if character.is_ascii_alphanumeric() {
            if separator && !output.is_empty() {
                output.push('-');
            }
            output.push(character.to_ascii_lowercase());
            separator = false;
        } else {
            separator = true;
        }
    }
    output.trim_matches('-').chars().take(64).collect()
}

fn safe_relative(root: &Path, value: &str) -> Result<PathBuf> {
    let substituted = value
        .replace("${extensionPath}", ".")
        .replace("${workspacePath}", ".");
    let relative = Path::new(&substituted);
    if relative.is_absolute()
        || relative.components().any(|component| {
            matches!(
                component,
                Component::ParentDir | Component::RootDir | Component::Prefix(_)
            )
        })
    {
        bail!("component path '{value}' escapes the plugin root");
    }
    Ok(root.join(relative))
}

fn value_paths(root: &Path, value: Option<&Value>) -> Result<Vec<PathBuf>> {
    let values = match value {
        Some(Value::String(path)) => vec![path.as_str()],
        Some(Value::Array(paths)) => paths.iter().filter_map(Value::as_str).collect(),
        _ => Vec::new(),
    };
    values
        .into_iter()
        .map(|path| safe_relative(root, path))
        .collect()
}

fn existing_default(root: &Path, relative: &str) -> Vec<PathBuf> {
    let path = root.join(relative);
    path.exists().then_some(path).into_iter().collect()
}

fn read_json(path: &Path) -> Result<Value> {
    let metadata = std::fs::metadata(path)
        .with_context(|| format!("failed to inspect plugin manifest '{}'", path.display()))?;
    if metadata.len() > 1024 * 1024 {
        bail!(
            "plugin manifest '{}' exceeds the 1 MiB limit",
            path.display()
        );
    }
    let content = std::fs::read_to_string(path)
        .with_context(|| format!("failed to read plugin manifest '{}'", path.display()))?;
    serde_json::from_str(&content)
        .with_context(|| format!("invalid JSON plugin manifest '{}'", path.display()))
}

fn manifest_candidate(root: &Path) -> Option<(PluginEcosystem, PathBuf)> {
    [
        (PluginEcosystem::Zavora, ".zavora-plugin/plugin.json"),
        (PluginEcosystem::Codex, ".codex-plugin/plugin.json"),
        (PluginEcosystem::Claude, ".claude-plugin/plugin.json"),
        (PluginEcosystem::Gemini, "gemini-extension.json"),
    ]
    .into_iter()
    .map(|(ecosystem, relative)| (ecosystem, root.join(relative)))
    .find(|(_, path)| path.is_file())
}

fn infer_ecosystem(root: &Path, fallback: PluginEcosystem) -> PluginEcosystem {
    let rendered = root.to_string_lossy();
    if rendered.contains("/.grok/") {
        PluginEcosystem::Grok
    } else if rendered.contains("/.opencode/") {
        PluginEcosystem::OpenCode
    } else {
        fallback
    }
}

fn dedup_paths(paths: &mut Vec<PathBuf>) {
    let mut seen = BTreeSet::new();
    paths.retain(|path| seen.insert(path.clone()));
}

pub fn inspect_plugin_root(root: &Path) -> Result<PluginDescriptor> {
    let root = root
        .canonicalize()
        .with_context(|| format!("failed to resolve plugin root '{}'", root.display()))?;
    if !root.is_dir() {
        bail!("plugin root '{}' is not a directory", root.display());
    }

    let manifest_candidate = manifest_candidate(&root);
    let (ecosystem, manifest, value) = match manifest_candidate {
        Some((ecosystem, path)) => {
            let value = read_json(&path)?;
            (infer_ecosystem(&root, ecosystem), Some(path), value)
        }
        None if root.join("opencode.json").is_file() || root.join("package.json").is_file() => {
            let path = if root.join("opencode.json").is_file() {
                root.join("opencode.json")
            } else {
                root.join("package.json")
            };
            (
                PluginEcosystem::OpenCode,
                Some(path.clone()),
                read_json(&path)?,
            )
        }
        None => {
            let has_components = ["skills", "agents", "commands", "hooks", ".mcp.json"]
                .into_iter()
                .any(|relative| root.join(relative).exists());
            if !has_components {
                bail!(
                    "'{}' has no supported plugin manifest or declarative components",
                    root.display()
                );
            }
            (
                infer_ecosystem(&root, PluginEcosystem::Claude),
                None,
                Value::Object(Default::default()),
            )
        }
    };

    let fallback_name = root
        .file_name()
        .and_then(|name| name.to_str())
        .map(normalize_name)
        .unwrap_or_default();
    let name = value
        .get("name")
        .and_then(Value::as_str)
        .map(normalize_name)
        .filter(|name| !name.is_empty())
        .unwrap_or(fallback_name);
    if !is_valid_name(&name) {
        bail!("plugin name '{name}' must be lowercase kebab-case and at most 64 characters");
    }

    let mut components = PluginComponents {
        skill_roots: value_paths(&root, value.get("skills"))?,
        ..PluginComponents::default()
    };
    if components.skill_roots.is_empty() {
        components
            .skill_roots
            .extend(existing_default(&root, "skills"));
    }
    components.agent_roots = value_paths(&root, value.get("agents"))?;
    components
        .agent_roots
        .extend(existing_default(&root, "agents"));
    components.command_roots = value_paths(&root, value.get("commands"))?;
    components
        .command_roots
        .extend(existing_default(&root, "commands"));
    components.hook_files = value_paths(&root, value.get("hooks"))?;
    components
        .hook_files
        .extend(existing_default(&root, "hooks/hooks.json"));

    match value.get("mcpServers") {
        Some(Value::String(path)) => components.mcp_files.push(safe_relative(&root, path)?),
        Some(Value::Object(_)) => components.inline_mcp = value.get("mcpServers").cloned(),
        _ => components
            .mcp_files
            .extend(existing_default(&root, ".mcp.json")),
    }
    components.app_files = value_paths(&root, value.get("apps"))?;
    components
        .app_files
        .extend(existing_default(&root, ".app.json"));

    for directory in [root.join("plugins"), root.join("src")] {
        if !directory.is_dir() {
            continue;
        }
        for entry in ignore::WalkBuilder::new(directory)
            .max_depth(Some(2))
            .build()
            .filter_map(std::result::Result::ok)
        {
            let path = entry.path();
            if entry.file_type().is_some_and(|kind| kind.is_file())
                && path
                    .extension()
                    .and_then(|ext| ext.to_str())
                    .is_some_and(|ext| {
                        matches!(
                            ext.to_ascii_lowercase().as_str(),
                            "js" | "mjs" | "cjs" | "ts"
                        )
                    })
            {
                components.executable_entrypoints.push(path.to_path_buf());
            }
        }
    }
    for paths in [
        &mut components.skill_roots,
        &mut components.agent_roots,
        &mut components.command_roots,
        &mut components.hook_files,
        &mut components.mcp_files,
        &mut components.app_files,
        &mut components.executable_entrypoints,
    ] {
        dedup_paths(paths);
    }

    let mut warnings = Vec::new();
    for (kind, paths) in [
        ("skill", &components.skill_roots),
        ("agent", &components.agent_roots),
        ("command", &components.command_roots),
        ("hook", &components.hook_files),
        ("MCP", &components.mcp_files),
        ("app", &components.app_files),
    ] {
        for path in paths.iter().filter(|path| path.exists()) {
            let canonical = path.canonicalize().with_context(|| {
                format!(
                    "failed to resolve declared {kind} component '{}'",
                    path.display()
                )
            })?;
            if !canonical.starts_with(&root) {
                bail!(
                    "declared {kind} component '{}' resolves outside plugin root '{}'",
                    path.display(),
                    root.display()
                );
            }
        }
        for path in paths.iter().filter(|path| !path.exists()) {
            warnings.push(format!(
                "declared {kind} component '{}' does not exist",
                path.display()
            ));
        }
    }
    if !components.executable_entrypoints.is_empty() {
        warnings.push(
            "executable JavaScript/TypeScript entrypoints require an explicit trusted runtime and are not auto-executed"
                .to_string(),
        );
    }

    let display_name = value
        .pointer("/interface/displayName")
        .or_else(|| value.get("displayName"))
        .and_then(Value::as_str)
        .unwrap_or(&name)
        .to_string();
    Ok(PluginDescriptor {
        name,
        display_name,
        version: value
            .get("version")
            .and_then(Value::as_str)
            .map(str::to_string),
        description: value
            .get("description")
            .and_then(Value::as_str)
            .map(str::to_string),
        ecosystem,
        root,
        manifest,
        enabled: true,
        managed: false,
        linked: false,
        source: None,
        components,
        warnings,
    })
}

fn scan_children(base: &Path, ecosystem: PluginEcosystem, output: &mut Vec<PluginDescriptor>) {
    let Ok(entries) = std::fs::read_dir(base) else {
        return;
    };
    let mut entries = entries
        .filter_map(std::result::Result::ok)
        .collect::<Vec<_>>();
    entries.sort_by_key(|entry| entry.file_name());
    for entry in entries {
        let path = entry.path();
        if !path.is_dir() {
            continue;
        }
        if let Ok(mut descriptor) = inspect_plugin_root(&path) {
            if descriptor.ecosystem == PluginEcosystem::Claude {
                descriptor.ecosystem = ecosystem;
            }
            output.push(descriptor);
        }
    }
}

fn state_records() -> Result<Vec<PluginRecord>> {
    let mut records = Vec::new();
    for scope in [ExtensionScope::User, ExtensionScope::Workspace] {
        let paths = plugin_paths(scope)?;
        records.extend(load_state(&paths.state)?.plugins);
    }
    Ok(records)
}

pub fn discover_plugins() -> Result<Vec<PluginDescriptor>> {
    let cwd = std::env::current_dir().context("failed to resolve current workspace")?;
    let home = std::env::var_os("HOME").map(PathBuf::from);
    let mut descriptors = Vec::new();

    if let Some(home) = &home {
        for (relative, ecosystem) in [
            (".zavora/plugins", PluginEcosystem::Zavora),
            (".gemini/extensions", PluginEcosystem::Gemini),
            (".grok/plugins", PluginEcosystem::Grok),
        ] {
            scan_children(&home.join(relative), ecosystem, &mut descriptors);
        }
    }
    for (relative, ecosystem) in [
        (".zavora/plugins", PluginEcosystem::Zavora),
        (".agents/plugins", PluginEcosystem::Codex),
        (".claude/plugins", PluginEcosystem::Claude),
        (".gemini/extensions", PluginEcosystem::Gemini),
        (".grok/plugins", PluginEcosystem::Grok),
    ] {
        scan_children(&cwd.join(relative), ecosystem, &mut descriptors);
    }

    let records = state_records()?;
    for record in &records {
        let mut descriptor =
            inspect_plugin_root(&record.path).unwrap_or_else(|error| PluginDescriptor {
                name: record.name.clone(),
                display_name: record.name.clone(),
                version: None,
                description: None,
                ecosystem: record.ecosystem,
                root: record.path.clone(),
                manifest: None,
                enabled: record.enabled,
                managed: true,
                linked: record.linked,
                source: Some(record.source.clone()),
                components: PluginComponents::default(),
                warnings: vec![format!(
                    "installed plugin is unavailable or invalid: {error}"
                )],
            });
        descriptor.enabled = record.enabled;
        descriptor.managed = true;
        descriptor.linked = record.linked;
        descriptor.source = Some(record.source.clone());
        descriptor.ecosystem = record.ecosystem;
        descriptors.push(descriptor);
    }

    let record_by_path = records
        .iter()
        .filter_map(|record| record.path.canonicalize().ok().map(|path| (path, record)))
        .collect::<BTreeMap<_, _>>();
    let mut by_path = BTreeMap::<PathBuf, PluginDescriptor>::new();
    for mut descriptor in descriptors {
        if let Some(record) = record_by_path.get(&descriptor.root) {
            descriptor.enabled = record.enabled;
            descriptor.managed = true;
            descriptor.linked = record.linked;
            descriptor.source = Some(record.source.clone());
            descriptor.ecosystem = record.ecosystem;
        }
        by_path.insert(descriptor.root.clone(), descriptor);
    }
    let mut values = by_path.into_values().collect::<Vec<_>>();
    values.sort_by(|left, right| left.name.cmp(&right.name).then(left.root.cmp(&right.root)));
    Ok(values)
}

pub fn enabled_plugin_skill_roots() -> Result<Vec<(String, PathBuf, bool)>> {
    let mut roots = Vec::new();
    for plugin in discover_plugins()?
        .into_iter()
        .filter(|plugin| plugin.enabled)
    {
        for root in plugin
            .components
            .skill_roots
            .into_iter()
            .filter(|path| path.is_dir())
        {
            roots.push((plugin.name.clone(), root, true));
        }
        for root in plugin
            .components
            .command_roots
            .into_iter()
            .filter(|path| path.exists())
        {
            let root = if root.is_file() {
                root.parent().unwrap_or(&root).to_path_buf()
            } else {
                root
            };
            roots.push((plugin.name.clone(), root, false));
        }
    }
    Ok(roots)
}

fn markdown_files(path: &Path) -> Vec<PathBuf> {
    if path.is_file() {
        return path
            .extension()
            .and_then(|extension| extension.to_str())
            .is_some_and(|extension| extension.eq_ignore_ascii_case("md"))
            .then_some(path.to_path_buf())
            .into_iter()
            .collect();
    }
    if !path.is_dir() {
        return Vec::new();
    }
    let mut files = ignore::WalkBuilder::new(path)
        .max_depth(Some(3))
        .hidden(false)
        .build()
        .filter_map(std::result::Result::ok)
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

fn markdown_frontmatter(content: &str) -> (BTreeMap<String, String>, String) {
    let Some(rest) = content.strip_prefix("---") else {
        return (BTreeMap::new(), content.trim().to_string());
    };
    let Some((frontmatter, body)) = rest.split_once("\n---") else {
        return (BTreeMap::new(), content.trim().to_string());
    };
    let values = frontmatter
        .lines()
        .filter_map(|line| line.split_once(':'))
        .map(|(key, value)| {
            (
                key.trim().to_ascii_lowercase(),
                value.trim().trim_matches(['\'', '"']).to_string(),
            )
        })
        .collect();
    (
        values,
        body.trim_start_matches(['\r', '\n']).trim().to_string(),
    )
}

pub fn enabled_plugin_agents() -> Result<HashMap<String, crate::config::ResolvedAgent>> {
    let mut agents = HashMap::new();
    for plugin in discover_plugins()?
        .into_iter()
        .filter(|plugin| plugin.enabled)
    {
        for root in &plugin.components.agent_roots {
            for path in markdown_files(root) {
                let content = match std::fs::read_to_string(&path) {
                    Ok(content) => content,
                    Err(error) => {
                        tracing::warn!(plugin = %plugin.name, path = %path.display(), %error, "plugin agent unavailable");
                        continue;
                    }
                };
                let (frontmatter, body) = markdown_frontmatter(&content);
                if body.is_empty() {
                    continue;
                }
                let local_name = frontmatter
                    .get("name")
                    .map(|name| normalize_name(name))
                    .filter(|name| !name.is_empty())
                    .or_else(|| {
                        path.file_stem()
                            .and_then(|name| name.to_str())
                            .map(normalize_name)
                    })
                    .unwrap_or_else(|| "agent".to_string());
                let name = format!("{}:{local_name}", plugin.name);
                agents.insert(
                    name.clone(),
                    crate::config::ResolvedAgent {
                        name,
                        source: crate::config::AgentSource::Plugin,
                        config: crate::config::AgentFileConfig {
                            description: frontmatter.get("description").cloned(),
                            instruction: Some(body),
                            provider: None,
                            // Keep imported agents portable across Zavora providers. A
                            // competitor-specific model hint must not silently replace
                            // the active worker route.
                            model: None,
                            tool_confirmation_mode: None,
                            resource_paths: vec![plugin.root.display().to_string()],
                            allow_tools: Vec::new(),
                            deny_tools: Vec::new(),
                            hooks: HashMap::new(),
                        },
                    },
                );
            }
        }
    }
    Ok(agents)
}

fn mcp_map_from_value(value: &Value) -> Option<&serde_json::Map<String, Value>> {
    value
        .get("mcpServers")
        .and_then(Value::as_object)
        .or_else(|| value.as_object())
}

fn parse_mcp_servers(plugin: &PluginDescriptor, value: &Value) -> Vec<McpServerConfig> {
    let Some(map) = mcp_map_from_value(value) else {
        return Vec::new();
    };
    map.iter()
        .filter_map(|(name, config)| {
            let object = config.as_object()?;
            let command = object
                .get("command")
                .and_then(Value::as_str)
                .map(|command| command.replace("${extensionPath}", &plugin.root.to_string_lossy()));
            let endpoint = object
                .get("url")
                .or_else(|| object.get("endpoint"))
                .and_then(Value::as_str)
                .unwrap_or_default()
                .replace("${extensionPath}", &plugin.root.to_string_lossy());
            if command.is_none() && endpoint.is_empty() {
                return None;
            }
            let args = object
                .get("args")
                .and_then(Value::as_array)
                .into_iter()
                .flatten()
                .filter_map(Value::as_str)
                .map(|arg| arg.replace("${extensionPath}", &plugin.root.to_string_lossy()))
                .collect();
            let env = object
                .get("env")
                .and_then(Value::as_object)
                .into_iter()
                .flat_map(|env| env.iter())
                .filter_map(|(key, value)| {
                    value.as_str().map(|value| (key.clone(), value.to_string()))
                })
                .collect::<HashMap<_, _>>();
            Some(McpServerConfig {
                name: format!("{}:{name}", plugin.name),
                endpoint,
                command,
                args,
                env,
                enabled: Some(true),
                timeout_secs: object.get("timeout_secs").and_then(Value::as_u64),
                auth_bearer_env: None,
                tool_allowlist: Vec::new(),
                tool_aliases: HashMap::new(),
                oauth: None,
            })
        })
        .collect()
}

pub fn enabled_plugin_mcp_servers() -> Result<Vec<McpServerConfig>> {
    let mut servers = Vec::new();
    for plugin in discover_plugins()?
        .into_iter()
        .filter(|plugin| plugin.enabled)
    {
        if let Some(value) = &plugin.components.inline_mcp {
            servers.extend(parse_mcp_servers(&plugin, value));
        }
        for path in &plugin.components.mcp_files {
            match read_json(path) {
                Ok(value) => servers.extend(parse_mcp_servers(&plugin, &value)),
                Err(error) => {
                    tracing::warn!(plugin = %plugin.name, %error, "plugin MCP manifest unavailable")
                }
            }
        }
    }
    let mut by_name = BTreeMap::new();
    for server in servers {
        by_name.insert(server.name.clone(), server);
    }
    Ok(by_name.into_values().collect())
}

pub(crate) fn run_git(arguments: &[&str], context: &str) -> Result<()> {
    let status = Command::new("git")
        .args(arguments)
        .status()
        .with_context(|| format!("failed to start git while {context}"))?;
    if !status.success() {
        bail!("git failed while {context} (exit status {status})");
    }
    Ok(())
}

pub(crate) fn copy_tree(source: &Path, target: &Path) -> Result<()> {
    std::fs::create_dir_all(target)
        .with_context(|| format!("failed to create plugin directory '{}'", target.display()))?;
    for entry in std::fs::read_dir(source)
        .with_context(|| format!("failed to read source directory '{}'", source.display()))?
    {
        let entry = entry?;
        let file_type = entry.file_type()?;
        let destination = target.join(entry.file_name());
        if file_type.is_symlink() {
            bail!(
                "plugin source contains unsupported symlink '{}'",
                entry.path().display()
            );
        }
        if file_type.is_dir() {
            copy_tree(&entry.path(), &destination)?;
        } else if file_type.is_file() {
            std::fs::copy(entry.path(), &destination).with_context(|| {
                format!("failed to copy plugin file '{}'", entry.path().display())
            })?;
        }
    }
    Ok(())
}

pub(crate) fn replace_directory(staging: &Path, target: &Path) -> Result<()> {
    let file_name = target
        .file_name()
        .and_then(|name| name.to_str())
        .context("managed package target has no file name")?;
    let backup = target.with_file_name(format!(".{file_name}.backup"));
    if backup.exists() {
        bail!(
            "stale package update backup exists at '{}'",
            backup.display()
        );
    }
    std::fs::rename(target, &backup)
        .with_context(|| format!("failed to stage existing package '{}'", target.display()))?;
    if let Err(error) = std::fs::rename(staging, target) {
        // Requirement 9.10: if the rollback also fails, say exactly what was
        // left behind. Discarding this error left the package existing only as
        // a hidden backup, and the next run wedged on the stale-backup bail
        // above with no explanation of how it got there.
        if let Err(rollback_error) = std::fs::rename(&backup, target) {
            return Err(error).with_context(|| {
                format!(
                    "failed to activate updated package '{}', and rollback failed: {rollback_error}. \
                     The previous version is preserved at '{}'; move it back to '{}' to recover.",
                    target.display(),
                    backup.display(),
                    target.display()
                )
            });
        }
        return Err(error).with_context(|| {
            format!(
                "failed to activate updated package '{}'; the previous version was restored",
                target.display()
            )
        });
    }
    std::fs::remove_dir_all(&backup)
        .with_context(|| format!("failed to remove package backup '{}'", backup.display()))
}

pub(crate) fn is_git_source(source: &str) -> bool {
    source.starts_with("https://")
        || source.starts_with("ssh://")
        || source.starts_with("git@")
        || source.ends_with(".git")
}

pub fn install_plugin(source: &str, scope: ExtensionScope, link: bool) -> Result<PluginDescriptor> {
    let paths = plugin_paths(scope)?;
    let mut temporary = None;
    let source_root = if is_git_source(source) {
        if link {
            bail!("--link requires a local directory; Git sources are installed as managed copies");
        }
        let temp = tempfile::Builder::new()
            .prefix("zavora-plugin-")
            .tempdir()?;
        let destination = temp.path().join("source");
        run_git(
            &[
                "clone",
                "--depth",
                "1",
                source,
                destination.to_string_lossy().as_ref(),
            ],
            "cloning plugin source",
        )?;
        temporary = Some(temp);
        destination
    } else {
        PathBuf::from(source)
            .canonicalize()
            .with_context(|| format!("failed to resolve plugin source '{source}'"))?
    };
    let stored_source = if is_git_source(source) {
        source.to_string()
    } else {
        source_root.display().to_string()
    };
    let inspected = inspect_plugin_root(&source_root)?;
    let source_ecosystem = inspected.ecosystem;
    let mut state = load_state(&paths.state)?;
    if state
        .plugins
        .iter()
        .any(|record| record.name == inspected.name)
    {
        bail!(
            "plugin '{}' is already installed in {} scope; update or uninstall it first",
            inspected.name,
            scope
        );
    }

    let installed_path = if link {
        source_root.clone()
    } else {
        let target = paths.install_root.join(&inspected.name);
        if target.exists() {
            bail!("plugin target '{}' already exists", target.display());
        }
        if is_git_source(source) {
            std::fs::create_dir_all(&paths.install_root)?;
            run_git(
                &[
                    "clone",
                    "--depth",
                    "1",
                    source,
                    target.to_string_lossy().as_ref(),
                ],
                "installing plugin",
            )?;
        } else {
            copy_tree(&source_root, &target)?;
        }
        target.canonicalize().unwrap_or(target)
    };
    drop(temporary);

    let installed = inspect_plugin_root(&installed_path)?;
    state.plugins.push(PluginRecord {
        name: installed.name.clone(),
        path: installed.root.clone(),
        source: stored_source.clone(),
        enabled: true,
        linked: link,
        ecosystem: source_ecosystem,
    });
    state
        .plugins
        .sort_by(|left, right| left.name.cmp(&right.name));
    save_state(&paths.state, &state)?;
    Ok(PluginDescriptor {
        ecosystem: source_ecosystem,
        managed: true,
        linked: link,
        source: Some(stored_source),
        ..installed
    })
}

pub fn set_plugin_enabled(name: &str, scope: ExtensionScope, enabled: bool) -> Result<()> {
    let paths = plugin_paths(scope)?;
    let mut state = load_state(&paths.state)?;
    let record = state
        .plugins
        .iter_mut()
        .find(|record| record.name == name)
        .with_context(|| format!("plugin '{name}' is not installed in {scope} scope"))?;
    record.enabled = enabled;
    save_state(&paths.state, &state)
}

pub fn update_plugins(name: Option<&str>, scope: ExtensionScope) -> Result<Vec<String>> {
    let paths = plugin_paths(scope)?;
    let state = load_state(&paths.state)?;
    let records = state
        .plugins
        .iter()
        .filter(|record| name.is_none_or(|name| record.name == name))
        .collect::<Vec<_>>();
    if records.is_empty() {
        bail!("no matching plugin is installed in {scope} scope");
    }
    let mut updated = Vec::new();
    for record in records {
        if record.linked {
            inspect_plugin_root(&record.path)?;
            updated.push(format!("{} (linked; validated)", record.name));
        } else if record.path.join(".git").is_dir() {
            run_git(
                &[
                    "-C",
                    record.path.to_string_lossy().as_ref(),
                    "pull",
                    "--ff-only",
                ],
                &format!("updating plugin '{}'", record.name),
            )?;
            inspect_plugin_root(&record.path)?;
            updated.push(record.name.clone());
        } else {
            let source = PathBuf::from(&record.source);
            if !source.is_dir() {
                bail!(
                    "plugin '{}' was copied from '{}' which is no longer available",
                    record.name,
                    source.display()
                );
            }
            let staging = paths.install_root.join(format!(".{}.update", record.name));
            if staging.exists() {
                std::fs::remove_dir_all(&staging)?;
            }
            copy_tree(&source, &staging)?;
            inspect_plugin_root(&staging)?;
            replace_directory(&staging, &record.path)?;
            updated.push(record.name.clone());
        }
    }
    Ok(updated)
}

pub fn uninstall_plugin(name: &str, scope: ExtensionScope) -> Result<bool> {
    let paths = plugin_paths(scope)?;
    let mut state = load_state(&paths.state)?;
    let index = state
        .plugins
        .iter()
        .position(|record| record.name == name)
        .with_context(|| format!("plugin '{name}' is not installed in {scope} scope"))?;
    let record = state.plugins.remove(index);
    let install_root = paths.install_root.canonicalize().unwrap_or_else(|_| {
        std::env::current_dir()
            .unwrap_or_default()
            .join(&paths.install_root)
    });
    let record_path = record
        .path
        .canonicalize()
        .unwrap_or_else(|_| record.path.clone());
    let removed_files = !record.linked && record_path.starts_with(install_root);
    if removed_files && record.path.is_dir() {
        std::fs::remove_dir_all(&record.path)
            .with_context(|| format!("failed to remove plugin '{}'", record.path.display()))?;
    }
    save_state(&paths.state, &state)?;
    Ok(removed_files)
}

pub fn format_plugins_markdown() -> Result<String> {
    let plugins = discover_plugins()?;
    if plugins.is_empty() {
        return Ok("## Plugins\n\nNo plugins or extensions discovered.".to_string());
    }
    let mut output = format!("## Plugins ({})\n\n", plugins.len());
    for plugin in plugins {
        let state = if plugin.enabled {
            "enabled"
        } else {
            "disabled"
        };
        output.push_str(&format!(
            "- `{}` — {} · {} · {} skill root(s) · {} MCP source(s)\n  `{}`\n",
            plugin.name,
            plugin.ecosystem,
            state,
            plugin.components.skill_roots.len(),
            plugin.components.mcp_files.len() + usize::from(plugin.components.inline_mcp.is_some()),
            plugin.root.display()
        ));
    }
    Ok(output)
}

pub fn doctor_report() -> Result<Value> {
    let plugins = discover_plugins()?;
    let enabled = plugins.iter().filter(|plugin| plugin.enabled).count();
    let mut warnings = plugins
        .iter()
        .flat_map(|plugin| {
            plugin
                .warnings
                .iter()
                .map(move |warning| json!({"plugin": plugin.name, "warning": warning}))
        })
        .collect::<Vec<_>>();
    let mut names = BTreeMap::<&str, usize>::new();
    for plugin in &plugins {
        *names.entry(&plugin.name).or_default() += 1;
    }
    warnings.extend(
        names
            .into_iter()
            .filter(|(_, count)| *count > 1)
            .map(|(name, count)| {
                json!({"plugin": name, "warning": format!("{count} installations share this name; lookup and skill namespaces are ambiguous")})
            }),
    );
    Ok(json!({
        "status": if warnings.is_empty() { "ready" } else { "warning" },
        "plugin_count": plugins.len(),
        "enabled_count": enabled,
        "warnings": warnings,
        "runtime": {
            "skills": "ADK-Rust PluginManager / SkillInjector",
            "mcp": "normalized declarative manifests",
            "javascript_typescript": "discovered; explicit trusted runtime required"
        },
        "ecosystems": ["zavora", "codex", "claude", "gemini", "grok", "opencode"]
    }))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_codex_manifest_and_inline_mcp() {
        let temp = tempfile::tempdir().expect("temp directory");
        std::fs::create_dir_all(temp.path().join(".codex-plugin")).expect("manifest directory");
        std::fs::create_dir_all(temp.path().join("skills/review")).expect("skill directory");
        std::fs::write(
            temp.path().join(".codex-plugin/plugin.json"),
            r#"{"name":"quality","version":"1.0.0","skills":"./skills","mcpServers":{"docs":{"url":"https://example.test/mcp"}}}"#,
        )
        .expect("manifest");

        let plugin = inspect_plugin_root(temp.path()).expect("inspect plugin");
        assert_eq!(plugin.name, "quality");
        assert_eq!(plugin.ecosystem, PluginEcosystem::Codex);
        assert_eq!(plugin.components.skill_roots.len(), 1);
        assert!(plugin.components.inline_mcp.is_some());
        let servers = parse_mcp_servers(
            &plugin,
            plugin.components.inline_mcp.as_ref().expect("inline MCP"),
        );
        assert_eq!(servers[0].name, "quality:docs");
    }

    #[test]
    fn parses_gemini_extension_and_substitutes_root() {
        let temp = tempfile::tempdir().expect("temp directory");
        std::fs::write(
            temp.path().join("gemini-extension.json"),
            r#"{"name":"workspace-tools","mcpServers":{"local":{"command":"node","args":["${extensionPath}/server.js"]}}}"#,
        )
        .expect("manifest");
        let plugin = inspect_plugin_root(temp.path()).expect("inspect plugin");
        assert_eq!(plugin.ecosystem, PluginEcosystem::Gemini);
        let servers = parse_mcp_servers(
            &plugin,
            plugin.components.inline_mcp.as_ref().expect("inline MCP"),
        );
        assert!(servers[0].args[0].contains("server.js"));
        assert!(!servers[0].args[0].contains("${extensionPath}"));
    }

    #[test]
    fn rejects_component_path_traversal() {
        let temp = tempfile::tempdir().expect("temp directory");
        std::fs::create_dir_all(temp.path().join(".claude-plugin")).expect("manifest directory");
        std::fs::write(
            temp.path().join(".claude-plugin/plugin.json"),
            r#"{"name":"unsafe","skills":"../outside"}"#,
        )
        .expect("manifest");
        let error = inspect_plugin_root(temp.path()).expect_err("path must be rejected");
        assert!(error.to_string().contains("escapes the plugin root"));
    }

    #[test]
    fn parses_native_zavora_manifest() {
        let temp = tempfile::tempdir().expect("temp directory");
        std::fs::create_dir_all(temp.path().join(".zavora-plugin")).expect("manifest directory");
        std::fs::write(
            temp.path().join(".zavora-plugin/plugin.json"),
            r#"{"name":"native-tools","description":"Native package"}"#,
        )
        .expect("manifest");

        let plugin = inspect_plugin_root(temp.path()).expect("inspect plugin");
        assert_eq!(plugin.ecosystem, PluginEcosystem::Zavora);
        assert_eq!(plugin.name, "native-tools");
    }

    #[test]
    fn identifies_claude_packages_inside_grok_roots() {
        let temp = tempfile::tempdir().expect("temp directory");
        let root = temp.path().join(".grok/plugins/portable-tools");
        std::fs::create_dir_all(root.join(".claude-plugin")).expect("manifest directory");
        std::fs::write(
            root.join(".claude-plugin/plugin.json"),
            r#"{"name":"portable-tools"}"#,
        )
        .expect("manifest");

        let plugin = inspect_plugin_root(&root).expect("inspect plugin");
        assert_eq!(plugin.ecosystem, PluginEcosystem::Grok);
    }

    #[test]
    fn discovers_opencode_entrypoints_without_executing_them() {
        let temp = tempfile::tempdir().expect("temp directory");
        std::fs::create_dir_all(temp.path().join("plugins")).expect("plugin directory");
        std::fs::write(
            temp.path().join("package.json"),
            r#"{"name":"opencode-audit","version":"2.0.0"}"#,
        )
        .expect("manifest");
        std::fs::write(temp.path().join("plugins/audit.ts"), "export default {};")
            .expect("entrypoint");

        let plugin = inspect_plugin_root(temp.path()).expect("inspect plugin");
        assert_eq!(plugin.ecosystem, PluginEcosystem::OpenCode);
        assert_eq!(plugin.components.executable_entrypoints.len(), 1);
        assert!(
            plugin
                .warnings
                .iter()
                .any(|warning| warning.contains("not auto-executed"))
        );
    }

    #[test]
    fn plugin_state_round_trips_enabled_and_linked_flags() {
        let temp = tempfile::tempdir().expect("temp directory");
        let path = temp.path().join("plugins.toml");
        let state = PluginState {
            version: STATE_VERSION,
            plugins: vec![PluginRecord {
                name: "portable-tools".to_string(),
                path: temp.path().join("portable-tools"),
                source: "../portable-tools".to_string(),
                enabled: false,
                linked: true,
                ecosystem: PluginEcosystem::Claude,
            }],
        };
        save_state(&path, &state).expect("save state");
        let loaded = load_state(&path).expect("load state");
        assert_eq!(loaded.plugins.len(), 1);
        assert!(!loaded.plugins[0].enabled);
        assert!(loaded.plugins[0].linked);
    }
}
