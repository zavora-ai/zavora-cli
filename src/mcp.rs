use std::sync::Arc;
use std::time::Duration;

use adk_rust::prelude::*;
use adk_rust::ReadonlyContext;
use adk_tool::mcp::RefreshConfig;
use adk_tool::{McpAuth, McpHttpClientBuilder};
use anyhow::{Context, Result};

use crate::config::{McpServerConfig, RuntimeConfig};

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

pub async fn discover_mcp_tools_for_server(
    server: &McpServerConfig,
    retry_attempts: u32,
    retry_delay_ms: u64,
) -> Result<Vec<Arc<dyn Tool>>> {
    let mut builder = McpHttpClientBuilder::new(server.endpoint.clone())
        .timeout(Duration::from_secs(server.timeout_secs.unwrap_or(15)));
    if let Some(auth) = resolve_mcp_auth(server)? {
        builder = builder.with_auth(auth);
    }

    let mut toolset = builder
        .connect()
        .await
        .with_context(|| {
            format!(
                "failed to connect to MCP server '{}' at {}",
                server.name, server.endpoint
            )
        })?
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
                tracing::info!(
                    server = %server.name,
                    endpoint = %server.endpoint,
                    tools = tools.len(),
                    "MCP tools discovered"
                );
                all_tools.append(&mut tools);
            }
            Err(err) => {
                tracing::warn!(
                    server = %server.name,
                    endpoint = %server.endpoint,
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
        println!(
            "- {} endpoint={} timeout={}s auth_env={} allowlist={}",
            server.name,
            server.endpoint,
            server.timeout_secs.unwrap_or(15),
            auth,
            allowlist
        );
    }

    Ok(())
}

pub async fn run_mcp_discover(cfg: &RuntimeConfig, server_name: Option<String>) -> Result<()> {
    let servers = select_mcp_servers(cfg, server_name.as_deref())?;
    if servers.is_empty() {
        println!("No enabled MCP servers configured for discovery.");
        return Ok(());
    }

    let mut failures = 0usize;
    for server in servers {
        match discover_mcp_tools_for_server(
            &server,
            cfg.tool_retry_attempts,
            cfg.tool_retry_delay_ms,
        )
        .await
        {
            Ok(tools) => {
                println!(
                    "MCP server '{}' reachable. Discovered {} tool(s):",
                    server.name,
                    tools.len()
                );
                for tool in tools {
                    println!("- {}", tool.name());
                }
            }
            Err(err) => {
                failures += 1;
                eprintln!(
                    "[TOOLING] MCP discovery failed for '{}' ({}): {}",
                    server.name, server.endpoint, err
                );
            }
        }
    }

    if failures > 0 {
        return Err(anyhow::anyhow!(
            "MCP discovery completed with {} failure(s). Check endpoint/auth and retry.",
            failures
        ));
    }

    Ok(())
}
