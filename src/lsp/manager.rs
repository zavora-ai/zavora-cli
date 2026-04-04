//! LSP server lifecycle manager.

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::Arc;

use anyhow::{Context, Result};
use serde_json::{json, Value};
use tokio::process::Command;
use tokio::sync::Mutex;

use super::client::LspClient;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Language {
    Rust,
    TypeScript,
    Python,
    Go,
    Java,
    Ruby,
    Cpp,
}

impl Language {
    pub fn detect(path: &Path) -> Option<Self> {
        match path.extension()?.to_str()? {
            "rs" => Some(Self::Rust),
            "ts" | "tsx" | "js" | "jsx" | "mjs" | "cjs" => Some(Self::TypeScript),
            "py" | "pyi" => Some(Self::Python),
            "go" => Some(Self::Go),
            "java" => Some(Self::Java),
            "rb" => Some(Self::Ruby),
            "c" | "h" | "cpp" | "hpp" | "cc" | "cxx" => Some(Self::Cpp),
            _ => None,
        }
    }

    pub fn id(&self) -> &'static str {
        match self {
            Self::Rust => "rust",
            Self::TypeScript => "typescript",
            Self::Python => "python",
            Self::Go => "go",
            Self::Java => "java",
            Self::Ruby => "ruby",
            Self::Cpp => "cpp",
        }
    }

    pub fn language_id(&self) -> &'static str {
        match self {
            Self::Rust => "rust",
            Self::TypeScript => "typescript",
            Self::Python => "python",
            Self::Go => "go",
            Self::Java => "java",
            Self::Ruby => "ruby",
            Self::Cpp => "cpp",
        }
    }
}

#[derive(Debug, Clone, serde::Deserialize, serde::Serialize)]
pub struct LspServerConfig {
    pub command: String,
    #[serde(default)]
    pub args: Vec<String>,
}

#[derive(Debug, Clone, serde::Deserialize, serde::Serialize)]
pub struct LspConfig {
    #[serde(default)]
    pub servers: HashMap<String, LspServerConfig>,
}

struct ServerHandle {
    client: Arc<LspClient>,
    open_files: HashSet<PathBuf>,
    #[allow(dead_code)]
    restart_count: u32,
}

pub struct LspManager {
    servers: Mutex<HashMap<Language, ServerHandle>>,
    config: LspConfig,
    workspace_root: PathBuf,
}

impl LspManager {
    pub fn new(config: LspConfig, workspace_root: PathBuf) -> Self {
        Self {
            servers: Mutex::new(HashMap::new()),
            config,
            workspace_root,
        }
    }

    /// Send an LSP request for a file. Starts the server lazily if needed.
    pub async fn request(
        &self,
        file_path: &Path,
        method: &str,
        params: Value,
    ) -> Result<Value> {
        let lang = Language::detect(file_path)
            .ok_or_else(|| anyhow::anyhow!("unsupported file type: {}", file_path.display()))?;

        let client = self.ensure_server(lang).await?;

        // Ensure file is open
        self.ensure_file_open(&client, lang, file_path).await?;

        client.request(method, Some(params)).await
    }

    /// Notify that a file was modified externally (by file_edit/fs_write).
    pub async fn notify_file_changed(&self, file_path: &Path, new_content: &str) -> Result<()> {
        let lang = match Language::detect(file_path) {
            Some(l) => l,
            None => return Ok(()),
        };

        let servers = self.servers.lock().await;
        if let Some(handle) = servers.get(&lang) {
            if handle.open_files.contains(file_path) {
                let uri = path_to_uri(file_path);
                handle.client.notify(
                    "textDocument/didChange",
                    Some(json!({
                        "textDocument": { "uri": uri, "version": 1 },
                        "contentChanges": [{ "text": new_content }]
                    })),
                ).await?;
            }
        }
        Ok(())
    }

    /// Shutdown all servers gracefully.
    pub async fn shutdown_all(&self) {
        let mut servers = self.servers.lock().await;
        for (lang, handle) in servers.drain() {
            tracing::debug!(language = lang.id(), "shutting down LSP server");
            let _ = handle.client.request("shutdown", None).await;
            let _ = handle.client.notify("exit", None).await;
        }
    }

    async fn ensure_server(&self, lang: Language) -> Result<Arc<LspClient>> {
        let mut servers = self.servers.lock().await;

        if let Some(handle) = servers.get(&lang) {
            return Ok(handle.client.clone());
        }

        let server_config = self.config.servers.get(lang.id()).ok_or_else(|| {
            anyhow::anyhow!("no LSP server configured for {}", lang.id())
        })?;

        let client = self.start_server(lang, server_config).await?;
        let client = Arc::new(client);

        servers.insert(lang, ServerHandle {
            client: client.clone(),
            open_files: HashSet::new(),
            restart_count: 0,
        });

        Ok(client)
    }

    async fn start_server(&self, lang: Language, config: &LspServerConfig) -> Result<LspClient> {
        tracing::info!(language = lang.id(), command = %config.command, "starting LSP server");

        let mut child = Command::new(&config.command)
            .args(&config.args)
            .stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::null())
            .spawn()
            .with_context(|| format!("failed to spawn LSP server: {}", config.command))?;

        let stdin = child.stdin.take().context("no stdin")?;
        let stdout = child.stdout.take().context("no stdout")?;

        let client = LspClient::new(stdin, stdout);

        // Initialize
        let root_uri = path_to_uri(&self.workspace_root);
        let init_result = client.request("initialize", Some(json!({
            "processId": std::process::id(),
            "rootUri": root_uri,
            "capabilities": {
                "textDocument": {
                    "definition": { "dynamicRegistration": false },
                    "references": { "dynamicRegistration": false },
                    "hover": { "dynamicRegistration": false, "contentFormat": ["plaintext", "markdown"] },
                    "documentSymbol": { "dynamicRegistration": false },
                    "callHierarchy": { "dynamicRegistration": false }
                },
                "workspace": {
                    "symbol": { "dynamicRegistration": false }
                }
            }
        }))).await.context("LSP initialize failed")?;

        tracing::debug!(language = lang.id(), "LSP initialized: {}", init_result);

        client.notify("initialized", Some(json!({}))).await?;

        Ok(client)
    }

    async fn ensure_file_open(
        &self,
        client: &LspClient,
        lang: Language,
        file_path: &Path,
    ) -> Result<()> {
        let mut servers = self.servers.lock().await;
        let handle = servers.get_mut(&lang).context("server not running")?;

        if handle.open_files.contains(file_path) {
            return Ok(());
        }

        let content = tokio::fs::read_to_string(file_path)
            .await
            .with_context(|| format!("failed to read {}", file_path.display()))?;

        let uri = path_to_uri(file_path);
        client.notify("textDocument/didOpen", Some(json!({
            "textDocument": {
                "uri": uri,
                "languageId": lang.language_id(),
                "version": 1,
                "text": content
            }
        }))).await?;

        handle.open_files.insert(file_path.to_path_buf());
        Ok(())
    }
}

fn path_to_uri(path: &Path) -> String {
    format!("file://{}", path.display())
}

/// Load LSP config from .zavora/lsp.json or .kiro/settings/lsp.json.
pub fn load_lsp_config() -> Option<LspConfig> {
    let candidates = [".zavora/lsp.json", ".kiro/settings/lsp.json"];
    for path in &candidates {
        if let Ok(content) = std::fs::read_to_string(path) {
            if let Ok(config) = serde_json::from_str::<LspConfig>(&content) {
                return Some(config);
            }
        }
    }
    None
}

/// Generate a default LSP config by detecting available servers in PATH.
pub fn generate_default_config() -> LspConfig {
    let mut servers = HashMap::new();

    let checks: &[(&str, &str, &[&str])] = &[
        ("rust", "rust-analyzer", &[]),
        ("typescript", "typescript-language-server", &["--stdio"]),
        ("python", "pylsp", &[]),
        ("go", "gopls", &["serve"]),
        ("java", "jdtls", &[]),
        ("ruby", "solargraph", &["stdio"]),
        ("cpp", "clangd", &[]),
    ];

    for &(lang, cmd, args) in checks {
        if which_exists(cmd) {
            servers.insert(lang.to_string(), LspServerConfig {
                command: cmd.to_string(),
                args: args.iter().map(|s| s.to_string()).collect(),
            });
        }
    }

    LspConfig { servers }
}

fn which_exists(cmd: &str) -> bool {
    std::process::Command::new("which")
        .arg(cmd)
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .map(|s| s.success())
        .unwrap_or(false)
}
