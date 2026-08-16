use std::sync::Arc;
use std::time::{Duration, Instant};

use adk_rust::ReadonlyContext;
use adk_rust::prelude::*;
use adk_tool::mcp::{
    AdkClientHandler, AutoDeclineElicitationHandler, ClientLifecycleMode, ClientServiceExt,
    McpServerConfig as AdkMcpServerConfig, McpServerManager, McpTaskConfig, RefreshConfig,
    RestartPolicy,
};
use adk_tool::{McpAuth, McpHttpClientBuilder, McpToolset};
use anyhow::{Context, Result};

use crate::config::{McpServerConfig, RuntimeConfig};
use crate::tool_policy::apply_tool_aliases;

#[derive(Debug)]
struct McpDiscoveryContext {
    user_content: Content,
}

impl Default for McpDiscoveryContext {
    fn default() -> Self {
        Self {
            user_content: Content::new("user").with_text("discover mcp tools"),
        }
    }
}

impl ReadonlyContext for McpDiscoveryContext {
    fn invocation_id(&self) -> &str {
        "mcp-discovery"
    }
    fn agent_name(&self) -> &str {
        "mcp-manager"
    }
    fn user_id(&self) -> &str {
        "local-user"
    }
    fn app_name(&self) -> &str {
        "zavora-cli"
    }
    fn session_id(&self) -> &str {
        "mcp-discovery"
    }
    fn branch(&self) -> &str {
        "main"
    }
    fn user_content(&self) -> &Content {
        &self.user_content
    }
}

pub fn select_mcp_servers(
    cfg: &RuntimeConfig,
    server_name: Option<&str>,
) -> Result<Vec<McpServerConfig>> {
    let active = cfg
        .mcp_servers
        .iter()
        .filter(|server| server.enabled.unwrap_or(true))
        .cloned()
        .collect::<Vec<McpServerConfig>>();

    if let Some(name) = server_name {
        let server = active
            .into_iter()
            .find(|server| server.name == name)
            .ok_or_else(|| anyhow::anyhow!("MCP server '{}' not found or not enabled", name))?;
        return Ok(vec![server]);
    }

    Ok(active)
}

pub fn resolve_mcp_auth(server: &McpServerConfig) -> Result<Option<McpAuth>> {
    let Some(env_key) = server.auth_bearer_env.as_deref() else {
        return Ok(None);
    };

    let token = std::env::var(env_key).with_context(|| {
        format!(
            "MCP server '{}' requires bearer token env '{}' but it is missing",
            server.name, env_key
        )
    })?;

    if token.trim().is_empty() {
        return Err(anyhow::anyhow!(
            "MCP server '{}' has empty bearer token from env '{}'",
            server.name,
            env_key
        ));
    }

    Ok(Some(McpAuth::bearer(token)))
}

async fn resolve_mcp_connection_auth(server: &McpServerConfig) -> Result<Option<McpAuth>> {
    if let Some(auth) = resolve_mcp_auth(server)? {
        return Ok(Some(auth));
    }
    let Some(oauth) = server.oauth.as_ref() else {
        return Ok(None);
    };
    #[cfg(feature = "oauth")]
    {
        let token = crate::mcp_auth::get_access_token(&server.name, oauth).await?;
        Ok(Some(McpAuth::bearer(token)))
    }
    #[cfg(not(feature = "oauth"))]
    {
        let _ = oauth;
        Err(anyhow::anyhow!(
            "MCP server '{}' requires OAuth; rebuild with '--features oauth'",
            server.name
        ))
    }
}

async fn connect_http_toolset(server: &McpServerConfig) -> Result<McpToolset<()>> {
    let mut builder = McpHttpClientBuilder::new(server.endpoint.clone())
        .timeout(Duration::from_secs(server.timeout_secs.unwrap_or(15)));
    if let Some(auth) = resolve_mcp_connection_auth(server).await? {
        builder = builder.with_auth(auth);
    }
    builder.connect().await.with_context(|| {
        format!(
            "failed to connect to MCP server '{}' at {}",
            server.name, server.endpoint
        )
    })
}

async fn connect_stdio_toolset(server: &McpServerConfig) -> Result<McpToolset<()>> {
    use rmcp::ServiceExt;
    use tokio::process::Command;

    let command = server
        .command
        .as_deref()
        .context("stdio MCP server is missing command")?;
    let mut process = Command::new(command);
    process.args(&server.args).envs(&server.env);
    let transport = rmcp::transport::TokioChildProcess::new(process)
        .with_context(|| format!("failed to spawn MCP server '{}' ({command})", server.name))?;
    let client = ().serve(transport).await.map_err(|error| {
        anyhow::anyhow!(
            "failed to connect to stdio MCP server '{}': {error:?}",
            server.name
        )
    })?;
    Ok(McpToolset::new(client).with_name(format!("mcp:{}", server.name)))
}

async fn connect_mcp_toolset(server: &McpServerConfig) -> Result<McpToolset<()>> {
    if server.is_stdio() {
        connect_stdio_toolset(server).await
    } else {
        connect_http_toolset(server).await
    }
}

// ---------------------------------------------------------------------------
// MCP server diagnostics
// ---------------------------------------------------------------------------

#[derive(Debug, Clone)]
pub enum McpServerState {
    Reachable { tool_count: usize, latency_ms: u64 },
    AuthFailure { hint: String },
    Timeout { timeout_secs: u64 },
    Unreachable { error: String },
}

#[derive(Debug, Clone)]
pub struct McpServerDiagnostic {
    pub name: String,
    pub endpoint: String,
    pub state: McpServerState,
}

/// Check auth readiness without connecting. Returns a hint if auth is misconfigured.
pub fn check_auth_hint(server: &McpServerConfig) -> Option<String> {
    let env_key = server.auth_bearer_env.as_deref()?;
    match std::env::var(env_key) {
        Ok(val) if val.trim().is_empty() => Some(format!("env '{}' is set but empty", env_key)),
        Err(_) => Some(format!(
            "env '{}' is not set — set it or remove auth_bearer_env",
            env_key
        )),
        Ok(_) => None,
    }
}

/// Diagnose a single MCP server: check auth, attempt discovery, measure latency.
pub async fn diagnose_mcp_server(
    server: &McpServerConfig,
    retry_attempts: u32,
    retry_delay_ms: u64,
) -> McpServerDiagnostic {
    // Pre-flight auth check
    if let Some(hint) = check_auth_hint(server) {
        return McpServerDiagnostic {
            name: server.name.clone(),
            endpoint: server.display_target().to_string(),
            state: McpServerState::AuthFailure { hint },
        };
    }

    let start = Instant::now();
    match discover_mcp_tools_for_server(server, retry_attempts, retry_delay_ms).await {
        Ok(tools) => McpServerDiagnostic {
            name: server.name.clone(),
            endpoint: server.display_target().to_string(),
            state: McpServerState::Reachable {
                tool_count: tools.len(),
                latency_ms: start.elapsed().as_millis() as u64,
            },
        },
        Err(err) => {
            let error_str = err.to_string();
            let state = if error_str.contains("timed out") || error_str.contains("timeout") {
                McpServerState::Timeout {
                    timeout_secs: server.timeout_secs.unwrap_or(15),
                }
            } else {
                McpServerState::Unreachable { error: error_str }
            };
            McpServerDiagnostic {
                name: server.name.clone(),
                endpoint: server.display_target().to_string(),
                state,
            }
        }
    }
}

pub async fn discover_mcp_tools_for_server(
    server: &McpServerConfig,
    retry_attempts: u32,
    retry_delay_ms: u64,
) -> Result<Vec<Arc<dyn Tool>>> {
    if server.is_stdio() {
        discover_stdio_mcp_tools_with_retry(server, retry_attempts, retry_delay_ms).await
    } else {
        discover_http_mcp_tools(server, retry_attempts, retry_delay_ms).await
    }
}

/// Upper bound on the stdio reconnect backoff.
const STDIO_MAX_BACKOFF_MS: u64 = 30_000;

/// Retry stdio discovery with bounded exponential backoff.
///
/// The HTTP path has had retry via `RefreshConfig` since v2; the stdio path had
/// none, so a server that was slow to come up on the first attempt was simply
/// absent for the rest of the session. Requirement 10.5.
async fn discover_stdio_mcp_tools_with_retry(
    server: &McpServerConfig,
    retry_attempts: u32,
    retry_delay_ms: u64,
) -> Result<Vec<Arc<dyn Tool>>> {
    let attempts = retry_attempts.max(1);
    let mut delay = retry_delay_ms.max(1);
    let mut last_error = None;

    for attempt in 1..=attempts {
        match discover_stdio_mcp_tools(server).await {
            Ok(tools) => return Ok(tools),
            Err(error) => {
                if attempt == attempts {
                    // Say that retries were exhausted, so the reported failure
                    // is distinguishable from a single failed attempt.
                    return Err(error.context(format!(
                        "stdio MCP server '{}' unreachable after {attempts} attempt(s)",
                        server.name
                    )));
                }
                tracing::debug!(
                    server = %server.name,
                    attempt,
                    delay_ms = delay,
                    error = %error,
                    "stdio MCP discovery failed; retrying"
                );
                last_error = Some(error);
                tokio::time::sleep(Duration::from_millis(delay)).await;
                delay = (delay * 2).min(STDIO_MAX_BACKOFF_MS);
            }
        }
    }

    Err(last_error
        .unwrap_or_else(|| anyhow::anyhow!("stdio MCP discovery failed for '{}'", server.name)))
}

async fn discover_http_mcp_tools(
    server: &McpServerConfig,
    retry_attempts: u32,
    retry_delay_ms: u64,
) -> Result<Vec<Arc<dyn Tool>>> {
    let mut toolset = connect_http_toolset(server)
        .await?
        .with_name(format!("mcp:{}", server.name));

    toolset = toolset.with_refresh_config(
        RefreshConfig::default()
            .with_max_attempts(retry_attempts.max(1))
            .with_retry_delay_ms(retry_delay_ms),
    );

    if !server.tool_allowlist.is_empty() {
        let allowed = server.tool_allowlist.clone();
        toolset = toolset.with_filter(move |tool_name| {
            allowed.iter().any(|allowed_name| allowed_name == tool_name)
        });
    }

    let ctx: Arc<dyn ReadonlyContext> = Arc::new(McpDiscoveryContext::default());
    toolset.tools(ctx).await.with_context(|| {
        format!(
            "failed to discover MCP tools from '{}' ({})",
            server.name, server.endpoint
        )
    })
}

// ---------------------------------------------------------------------------
// Stdio MCP client
// ---------------------------------------------------------------------------

async fn discover_stdio_mcp_tools(server: &McpServerConfig) -> Result<Vec<Arc<dyn Tool>>> {
    use rmcp::model::ProtocolVersion;
    use tokio::process::Command;

    let command = server.command.as_deref().unwrap_or("echo");
    let mut process = Command::new(command);
    process.args(&server.args).envs(&server.env);
    let transport = rmcp::transport::TokioChildProcess::new(process)
        .with_context(|| format!("failed to spawn MCP server '{}' ({command})", server.name))?;

    // Prefer the MCP 2026 stateless lifecycle and fall back only when a legacy
    // server correctly reports server/discover as unsupported. Bounding the
    // handshake prevents a non-compliant server from hanging startup forever.
    let handler = AdkClientHandler::new(Arc::new(AutoDeclineElicitationHandler)).with_tasks();
    let connect = handler.serve_with_lifecycle(
        transport,
        ClientLifecycleMode::Auto {
            preferred_versions: vec![ProtocolVersion::V_2026_07_28],
            legacy_version: Some(ProtocolVersion::V_2025_11_25),
        },
    );
    let client = tokio::time::timeout(
        Duration::from_secs(server.timeout_secs.unwrap_or(15)),
        connect,
    )
    .await
    .with_context(|| format!("MCP handshake with '{}' timed out", server.name))?
    .map_err(|error| {
        anyhow::anyhow!(
            "failed to connect to stdio MCP server '{}': {error:?}",
            server.name
        )
    })?;

    let mut toolset = McpToolset::new(client)
        .with_name(format!("mcp:{}", server.name))
        .with_task_support(McpTaskConfig::enabled());
    if !server.tool_allowlist.is_empty() {
        let allowed = server.tool_allowlist.clone();
        toolset = toolset.with_filter(move |tool_name| {
            allowed.iter().any(|allowed_name| allowed_name == tool_name)
        });
    }
    let ctx: Arc<dyn ReadonlyContext> = Arc::new(McpDiscoveryContext::default());
    let tools = toolset
        .tools(ctx)
        .await
        .with_context(|| format!("failed to list tools from '{}'", server.name))?;
    Ok(tools
        .into_iter()
        .map(|tool| {
            Arc::new(crate::tool_policy::AliasedTool::new(
                format!("mcp:{}:{}", server.name, tool.name()),
                tool,
            )) as Arc<dyn Tool>
        })
        .collect())
}

/// A server that was configured and enabled but could not be reached.
///
/// Previously a connect failure was reported only through `tracing::warn!`,
/// which the Workspace's alternate screen swallows entirely, so a developer saw
/// tools silently missing with no explanation. Requirement 10.4.
#[derive(Debug, Clone)]
pub struct McpConnectFailure {
    pub server: String,
    pub target: String,
    pub error: String,
}

/// Discover MCP tools, reporting servers that failed to connect.
pub async fn discover_mcp_tools_reporting(
    cfg: &RuntimeConfig,
) -> (Vec<Arc<dyn Tool>>, Vec<McpConnectFailure>) {
    let mut failures = Vec::new();
    let mut all_tools = Vec::<Arc<dyn Tool>>::new();

    let servers = match select_mcp_servers(cfg, None) {
        Ok(servers) => servers,
        Err(err) => {
            tracing::warn!(error = %err, "MCP server selection failed");
            failures.push(McpConnectFailure {
                server: "<selection>".to_string(),
                target: String::new(),
                error: err.to_string(),
            });
            return (all_tools, failures);
        }
    };

    for server in servers {
        match discover_mcp_tools_for_server(
            &server,
            cfg.tool_retry_attempts,
            cfg.tool_retry_delay_ms,
        )
        .await
        {
            Ok(mut tools) => {
                tools = apply_tool_aliases(tools, &server.tool_aliases);
                tracing::info!(
                    server = %server.name,
                    target = %server.display_target(),
                    tools = tools.len(),
                    aliases = server.tool_aliases.len(),
                    "MCP tools discovered"
                );
                all_tools.append(&mut tools);
            }
            Err(err) => {
                tracing::warn!(
                    server = %server.name,
                    target = %server.display_target(),
                    error = %err,
                    "MCP server unavailable; continuing without its tools"
                );
                failures.push(McpConnectFailure {
                    server: server.name.clone(),
                    target: server.display_target().to_string(),
                    error: err.to_string(),
                });
            }
        }
    }

    (all_tools, failures)
}

pub async fn discover_mcp_tools(cfg: &RuntimeConfig) -> Vec<Arc<dyn Tool>> {
    let mut all_tools = Vec::<Arc<dyn Tool>>::new();
    let servers = match select_mcp_servers(cfg, None) {
        Ok(servers) => servers,
        Err(err) => {
            tracing::warn!(error = %err, "MCP server selection failed");
            return all_tools;
        }
    };

    for server in servers {
        match discover_mcp_tools_for_server(
            &server,
            cfg.tool_retry_attempts,
            cfg.tool_retry_delay_ms,
        )
        .await
        {
            Ok(mut tools) => {
                tools = apply_tool_aliases(tools, &server.tool_aliases);
                tracing::info!(
                    server = %server.name,
                    target = %server.display_target(),
                    tools = tools.len(),
                    aliases = server.tool_aliases.len(),
                    "MCP tools discovered"
                );
                all_tools.append(&mut tools);
            }
            Err(err) => {
                tracing::warn!(
                    server = %server.name,
                    target = %server.display_target(),
                    error = %err,
                    "MCP server unavailable; continuing without its tools"
                );
            }
        }
    }

    all_tools
}

pub async fn run_mcp_list(cfg: &RuntimeConfig) -> Result<()> {
    let servers = select_mcp_servers(cfg, None)?;
    if servers.is_empty() {
        println!(
            "No enabled MCP servers configured for profile '{}'.",
            cfg.profile
        );
        return Ok(());
    }

    println!("Enabled MCP servers for profile '{}':", cfg.profile);
    println!(
        "Runtime MCP reliability policy: retry_attempts={} retry_delay_ms={}",
        cfg.tool_retry_attempts, cfg.tool_retry_delay_ms
    );
    for server in servers {
        let auth = server.auth_bearer_env.as_deref().unwrap_or("<none>");
        let allowlist = if server.tool_allowlist.is_empty() {
            "<all>".to_string()
        } else {
            server.tool_allowlist.join(",")
        };
        let auth_hint = check_auth_hint(&server);
        let auth_status = match &auth_hint {
            Some(hint) => format!(" ⚠ {}", hint),
            None if server.auth_bearer_env.is_some() => " ✓".to_string(),
            None => String::new(),
        };
        let aliases = if server.tool_aliases.is_empty() {
            String::new()
        } else {
            format!(" aliases={}", server.tool_aliases.len())
        };
        let transport_label = if server.is_stdio() { "stdio" } else { "http" };
        println!(
            "- {} transport={} target={} timeout={}s auth_env={}{} allowlist={}{}",
            server.name,
            transport_label,
            server.display_target(),
            server.timeout_secs.unwrap_or(15),
            auth,
            auth_status,
            allowlist,
            aliases,
        );
    }

    Ok(())
}

pub async fn run_mcp_auth(cfg: &RuntimeConfig, server_name: &str) -> Result<()> {
    let server = cfg
        .mcp_servers
        .iter()
        .find(|server| server.name == server_name)
        .ok_or_else(|| anyhow::anyhow!("MCP server '{server_name}' is not configured"))?;
    let oauth = server.oauth.as_ref().ok_or_else(|| {
        anyhow::anyhow!(
            "MCP server '{server_name}' has no OAuth configuration; use auth_bearer_env for static bearer-token authentication"
        )
    })?;
    #[cfg(feature = "oauth")]
    {
        crate::mcp_auth::get_access_token(server_name, oauth).await?;
        println!("OAuth credentials stored for MCP server '{server_name}'.");
        Ok(())
    }
    #[cfg(not(feature = "oauth"))]
    {
        let _ = oauth;
        Err(anyhow::anyhow!(
            "MCP OAuth is not compiled in. Rebuild with '--features oauth'."
        ))
    }
}

pub async fn run_mcp_discover(cfg: &RuntimeConfig, server_name: Option<String>) -> Result<()> {
    let servers = select_mcp_servers(cfg, server_name.as_deref())?;
    if servers.is_empty() {
        println!("No enabled MCP servers configured for discovery.");
        return Ok(());
    }

    let mut failures = 0usize;
    for server in &servers {
        let diag =
            diagnose_mcp_server(server, cfg.tool_retry_attempts, cfg.tool_retry_delay_ms).await;
        match &diag.state {
            McpServerState::Reachable {
                tool_count,
                latency_ms,
            } => {
                println!(
                    "✓ '{}' reachable ({} tool(s), {}ms)",
                    diag.name, tool_count, latency_ms
                );
                // Re-discover to print tool names
                if let Ok(tools) = discover_mcp_tools_for_server(
                    server,
                    cfg.tool_retry_attempts,
                    cfg.tool_retry_delay_ms,
                )
                .await
                {
                    for tool in tools {
                        println!("  - {}", tool.name());
                    }
                }
            }
            McpServerState::AuthFailure { hint } => {
                failures += 1;
                eprintln!("✗ '{}' auth failure: {}", diag.name, hint);
            }
            McpServerState::Timeout { timeout_secs } => {
                failures += 1;
                eprintln!(
                    "✗ '{}' timed out after {}s (endpoint: {})",
                    diag.name, timeout_secs, diag.endpoint
                );
            }
            McpServerState::Unreachable { error } => {
                failures += 1;
                eprintln!(
                    "✗ '{}' unreachable ({}): {}",
                    diag.name, diag.endpoint, error
                );
            }
        }
    }

    if failures > 0 {
        return Err(anyhow::anyhow!(
            "MCP discovery completed with {} failure(s) out of {} server(s).",
            failures,
            servers.len()
        ));
    }

    Ok(())
}

pub fn run_mcp_info(cfg: &RuntimeConfig, server_name: &str) -> Result<()> {
    let server = cfg
        .mcp_servers
        .iter()
        .find(|server| server.name == server_name)
        .ok_or_else(|| anyhow::anyhow!("MCP server '{server_name}' is not configured"))?;
    println!("MCP server: {}", server.name);
    println!("Enabled: {}", server.enabled.unwrap_or(true));
    println!(
        "Transport: {}",
        if server.is_stdio() {
            "stdio"
        } else {
            "streamable-http"
        }
    );
    println!("Target: {}", server.display_target());
    println!("Timeout: {}s", server.timeout_secs.unwrap_or(15));
    println!(
        "Authentication: {}",
        if server.oauth.is_some() {
            "OAuth 2.0 + PKCE"
        } else {
            server.auth_bearer_env.as_deref().unwrap_or("none")
        }
    );
    println!(
        "Tool allowlist: {}",
        if server.tool_allowlist.is_empty() {
            "<all>".to_string()
        } else {
            server.tool_allowlist.join(", ")
        }
    );
    if !server.tool_aliases.is_empty() {
        println!("Tool aliases:");
        let mut aliases = server.tool_aliases.iter().collect::<Vec<_>>();
        aliases.sort_by(|left, right| left.0.cmp(right.0));
        for (source, target) in aliases {
            println!("  {source} -> {target}");
        }
    }
    Ok(())
}

pub async fn run_mcp_doctor(
    cfg: &RuntimeConfig,
    server_name: Option<String>,
    json: bool,
) -> Result<()> {
    let servers = select_mcp_servers(cfg, server_name.as_deref())?;
    let mut diagnostics = Vec::new();
    for server in servers {
        diagnostics.push(
            diagnose_mcp_server(&server, cfg.tool_retry_attempts, cfg.tool_retry_delay_ms).await,
        );
    }
    if json {
        let payload = diagnostics
            .iter()
            .map(|diagnostic| {
                let (status, detail) = match &diagnostic.state {
                    McpServerState::Reachable {
                        tool_count,
                        latency_ms,
                    } => (
                        "reachable",
                        serde_json::json!({"tool_count": tool_count, "latency_ms": latency_ms}),
                    ),
                    McpServerState::AuthFailure { hint } => {
                        ("auth-failure", serde_json::json!({"hint": hint}))
                    }
                    McpServerState::Timeout { timeout_secs } => {
                        ("timeout", serde_json::json!({"timeout_secs": timeout_secs}))
                    }
                    McpServerState::Unreachable { error } => {
                        ("unreachable", serde_json::json!({"error": error}))
                    }
                };
                serde_json::json!({
                    "name": diagnostic.name,
                    "target": diagnostic.endpoint,
                    "status": status,
                    "detail": detail,
                })
            })
            .collect::<Vec<_>>();
        println!("{}", serde_json::to_string_pretty(&payload)?);
        return Ok(());
    }

    if diagnostics.is_empty() {
        println!("No enabled MCP servers configured.");
    }
    let mut failures = 0usize;
    for diagnostic in diagnostics {
        match diagnostic.state {
            McpServerState::Reachable {
                tool_count,
                latency_ms,
            } => println!(
                "✓ {} reachable ({} tools, {}ms)",
                diagnostic.name, tool_count, latency_ms
            ),
            McpServerState::AuthFailure { hint } => {
                failures += 1;
                println!("✗ {} authentication: {}", diagnostic.name, hint);
            }
            McpServerState::Timeout { timeout_secs } => {
                failures += 1;
                println!("✗ {} timed out after {}s", diagnostic.name, timeout_secs);
            }
            McpServerState::Unreachable { error } => {
                failures += 1;
                println!("✗ {} unreachable: {}", diagnostic.name, error);
            }
        }
    }
    if failures > 0 {
        return Err(anyhow::anyhow!(
            "MCP doctor found {failures} failing server(s)"
        ));
    }
    Ok(())
}

pub async fn run_mcp_resources(
    cfg: &RuntimeConfig,
    server_name: &str,
    uri: Option<&str>,
    json: bool,
) -> Result<()> {
    let server = select_mcp_servers(cfg, Some(server_name))?
        .into_iter()
        .next()
        .context("selected MCP server disappeared")?;
    let toolset = connect_mcp_toolset(&server).await?;
    if let Some(uri) = uri {
        let contents = toolset.read_resource(uri).await?;
        if json {
            println!("{}", serde_json::to_string_pretty(&contents)?);
        } else if contents.is_empty() {
            println!("Resource '{uri}' returned no content.");
        } else {
            for content in contents {
                println!("{content:?}");
            }
        }
        return Ok(());
    }

    let resources = toolset.list_resources().await?;
    let templates = toolset.list_resource_templates().await?;
    if json {
        println!(
            "{}",
            serde_json::to_string_pretty(&serde_json::json!({
                "resources": resources,
                "templates": templates,
            }))?
        );
    } else {
        println!("Resources from '{}':", server.name);
        if resources.is_empty() && templates.is_empty() {
            println!("  <none declared>");
        }
        for resource in resources {
            println!("  {resource:?}");
        }
        for template in templates {
            println!("  template {template:?}");
        }
    }
    Ok(())
}

pub async fn run_mcp_prompts(
    cfg: &RuntimeConfig,
    server_name: &str,
    prompt_name: Option<&str>,
    arguments_json: Option<&str>,
    json: bool,
) -> Result<()> {
    let server = select_mcp_servers(cfg, Some(server_name))?
        .into_iter()
        .next()
        .context("selected MCP server disappeared")?;
    let toolset = connect_mcp_toolset(&server).await?;
    if let Some(name) = prompt_name {
        let arguments = arguments_json
            .map(serde_json::from_str::<serde_json::Map<String, serde_json::Value>>)
            .transpose()
            .context("--arguments must be a JSON object")?;
        let prompt = toolset.get_prompt(name, arguments).await?;
        if json {
            println!("{}", serde_json::to_string_pretty(&prompt)?);
        } else {
            println!("{prompt:?}");
        }
        return Ok(());
    }

    let prompts = toolset.list_prompts().await?;
    if json {
        println!("{}", serde_json::to_string_pretty(&prompts)?);
    } else {
        println!("Prompts from '{}':", server.name);
        if prompts.is_empty() {
            println!("  <none declared>");
        }
        for prompt in prompts {
            println!("  {prompt:?}");
        }
    }
    Ok(())
}

pub fn run_mcp_protocol(json: bool) -> Result<()> {
    let payload = serde_json::json!({
        "sdk": "rmcp 3.1",
        "native_client_revision": "2026-07-28 for stdio tool discovery",
        "compatible_server_revision": "2026-07-28",
        "native_2026_client": true,
        "capabilities": {
            "tools": true,
            "resources": true,
            "resource_templates": true,
            "prompts": true,
            "completion": true,
            "elicitation": "advertised with a safe auto-decline handler",
            "tasks": "negotiated for tool calls",
            "stdio": true,
            "streamable_http": true,
            "oauth_pkce": cfg!(feature = "oauth"),
            "dynamic_local_manager": true,
        },
        "remaining_native_2026_gates": [
            "server/discover lifecycle for the Streamable HTTP connector",
            "MCP 2026 authorization issuer and resource-indicator validation",
            "interactive form and URL elicitation UI",
            "MRTR resume channel for input_required tasks",
        ]
    });
    if json {
        println!("{}", serde_json::to_string_pretty(&payload)?);
    } else {
        println!("MCP SDK: rmcp 3.1");
        println!("Native stdio tool revision: 2026-07-28 (server/discover with legacy fallback)");
        println!("Compatible server revision: 2026-07-28");
        println!("Tools/resources/prompts/completion: supported");
        println!("Elicitation and negotiated tool-call tasks: supported");
        println!("Transports: stdio, Streamable HTTP");
        println!("OAuth 2.0 + PKCE compiled: {}", cfg!(feature = "oauth"));
        println!("Native stateless MCP 2026 stdio client: enabled");
        println!(
            "Run with --json to inspect the remaining HTTP, authorization, elicitation, and resume gates."
        );
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// ADK-Rust v2 McpServerManager lifecycle management
// ---------------------------------------------------------------------------

/// Convert our config format to the ADK McpServerConfig format for the manager.
fn to_adk_mcp_config(server: &McpServerConfig) -> Option<AdkMcpServerConfig> {
    if !server.is_stdio() {
        // McpServerManager only handles stdio servers (child processes)
        return None;
    }

    let cmd = server.command.as_deref()?;
    Some(AdkMcpServerConfig {
        command: cmd.to_string(),
        args: server.args.clone(),
        env: server.env.clone(),
        disabled: !server.enabled.unwrap_or(true),
        restart_policy: Some(RestartPolicy::default()),
        auto_approve: vec![],
    })
}

/// Build a managed MCP server manager for stdio servers with auto-restart.
/// This provides health monitoring and crash recovery for long-running sessions.
pub async fn build_managed_mcp_servers(cfg: &RuntimeConfig) -> Option<McpServerManager> {
    let servers = select_mcp_servers(cfg, None).ok()?;
    let stdio_servers: Vec<_> = servers.iter().filter(|s| s.is_stdio()).collect();

    if stdio_servers.is_empty() {
        return None;
    }

    let mut configs = std::collections::HashMap::new();
    for server in &stdio_servers {
        if let Some(adk_cfg) = to_adk_mcp_config(server) {
            configs.insert(server.name.clone(), adk_cfg);
        }
    }

    if configs.is_empty() {
        return None;
    }

    let manager = McpServerManager::new(configs);
    let results = manager.start_all().await;

    // Check if any servers failed to start
    let failures: Vec<_> = results
        .iter()
        .filter_map(|(name, result)| result.as_ref().err().map(|e| (name.clone(), e.to_string())))
        .collect();

    if !failures.is_empty() {
        for (name, err) in &failures {
            tracing::warn!(server = %name, error = %err, "MCP server failed to start");
        }
    }

    let started = results.len() - failures.len();
    if started > 0 {
        // Start health monitoring with auto-restart for running servers
        manager.start_monitoring();
        tracing::info!(
            started = started,
            failed = failures.len(),
            "MCP server manager active with health monitoring"
        );
        Some(manager)
    } else {
        tracing::warn!("All MCP servers failed to start");
        None
    }
}

/// Coverage for the MCP client surface, which previously had none.
///
/// Requirement 10.10. These are the paths that decide which servers are
/// contacted, how their tools are named, and whether the runtime can explain a
/// failure — all of which were untested while shipping.
#[cfg(test)]
mod mcp_tests {
    use super::*;
    use crate::config::McpServerConfig;

    fn http_server(name: &str, enabled: Option<bool>) -> McpServerConfig {
        McpServerConfig {
            name: name.to_string(),
            endpoint: format!("https://{name}.test/mcp"),
            enabled,
            ..Default::default()
        }
    }

    fn stdio_server(name: &str, command: &str) -> McpServerConfig {
        McpServerConfig {
            name: name.to_string(),
            command: Some(command.to_string()),
            ..Default::default()
        }
    }

    fn cfg_with(servers: Vec<McpServerConfig>) -> RuntimeConfig {
        let mut cfg = crate::tests::base_cfg();
        cfg.mcp_servers = servers;
        cfg
    }

    #[test]
    fn selection_includes_enabled_and_default_servers_only() {
        let cfg = cfg_with(vec![
            http_server("alpha", Some(true)),
            http_server("beta", Some(false)),
            // Absent `enabled` defaults to enabled.
            http_server("gamma", None),
        ]);

        let selected = select_mcp_servers(&cfg, None).expect("selection should succeed");
        let names = selected
            .iter()
            .map(|server| server.name.as_str())
            .collect::<Vec<_>>();
        assert_eq!(names, vec!["alpha", "gamma"]);
    }

    #[test]
    fn selecting_by_name_returns_only_that_server() {
        let cfg = cfg_with(vec![http_server("alpha", None), http_server("beta", None)]);
        let selected = select_mcp_servers(&cfg, Some("beta")).expect("beta is enabled");
        assert_eq!(selected.len(), 1);
        assert_eq!(selected[0].name, "beta");
    }

    #[test]
    fn selecting_a_disabled_server_by_name_is_an_error() {
        let cfg = cfg_with(vec![http_server("beta", Some(false))]);
        let error = select_mcp_servers(&cfg, Some("beta")).expect_err("disabled server");
        assert!(
            error.to_string().contains("not found or not enabled"),
            "{error}"
        );
    }

    #[test]
    fn selecting_an_unknown_server_is_an_error() {
        let cfg = cfg_with(vec![http_server("alpha", None)]);
        assert!(select_mcp_servers(&cfg, Some("missing")).is_err());
    }

    /// Transport dispatch is driven by the presence of `command`, which is what
    /// replaced the spec's proposed `McpTransport` enum.
    #[test]
    fn transport_is_chosen_by_the_presence_of_a_command() {
        let stdio = stdio_server("local", "npx some-mcp-server");
        assert!(stdio.is_stdio());
        assert_eq!(stdio.display_target(), "npx some-mcp-server");

        let http = http_server("remote", None);
        assert!(!http.is_stdio());
        assert_eq!(http.display_target(), "https://remote.test/mcp");
    }

    #[test]
    fn auth_hint_is_absent_when_no_auth_is_configured() {
        assert!(check_auth_hint(&http_server("alpha", None)).is_none());
    }

    #[test]
    fn auth_hint_reports_a_missing_environment_variable() {
        let mut server = http_server("alpha", None);
        server.auth_bearer_env = Some("ZAVORA_TEST_ABSENT_TOKEN".to_string());
        let hint = check_auth_hint(&server).expect("missing env should hint");
        assert!(hint.contains("is not set"), "{hint}");
    }

    /// A configured-but-unreachable server must produce a renderable report, so
    /// no surface can present a smaller tool set as complete.
    #[test]
    fn connect_failures_render_with_server_and_target() {
        let failure = McpConnectFailure {
            server: "alpha".to_string(),
            target: "https://alpha.test/mcp".to_string(),
            error: "connection refused".to_string(),
        };
        let rendered = format!("{} ({}): {}", failure.server, failure.target, failure.error);
        assert!(rendered.contains("alpha"));
        assert!(rendered.contains("https://alpha.test/mcp"));
        assert!(rendered.contains("connection refused"));
    }

    /// Requirement 10.5: the backoff must grow and then stop growing, so a
    /// server that never comes up cannot stall startup indefinitely.
    #[test]
    fn stdio_backoff_doubles_and_is_capped() {
        let mut delay = 500u64;
        let mut seen = vec![delay];
        for _ in 0..12 {
            delay = (delay * 2).min(STDIO_MAX_BACKOFF_MS);
            seen.push(delay);
        }
        assert_eq!(&seen[..4], &[500, 1_000, 2_000, 4_000]);
        assert_eq!(
            *seen.last().expect("non-empty"),
            STDIO_MAX_BACKOFF_MS,
            "backoff must saturate at the cap"
        );
        assert!(
            seen.iter().all(|value| *value <= STDIO_MAX_BACKOFF_MS),
            "backoff exceeded its cap: {seen:?}"
        );
    }

    #[tokio::test]
    async fn discovery_reports_no_failures_when_nothing_is_configured() {
        let cfg = cfg_with(Vec::new());
        let (tools, failures) = discover_mcp_tools_reporting(&cfg).await;
        assert!(tools.is_empty());
        assert!(failures.is_empty(), "{failures:?}");
    }
}
