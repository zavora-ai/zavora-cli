use std::sync::Arc;

use rmcp::model::{
    CallToolRequestParams, CallToolResult, Content, Implementation, ListToolsResult,
    PaginatedRequestParams, ServerCapabilities, ServerInfo,
};
use rmcp::{ErrorData as McpError, ServerHandler, ServiceExt, transport::stdio};
use serde_json::Value;

use crate::tools::build_builtin_tools;

/// MCP server that exposes zavora-cli's built-in tools over stdio.
pub struct ZavoraMcpServer {
    tools: Vec<Arc<dyn adk_rust::Tool>>,
}

impl ZavoraMcpServer {
    pub fn new() -> Self {
        Self {
            tools: build_builtin_tools(),
        }
    }

    fn adk_tool_to_mcp(&self, tool: &dyn adk_rust::Tool) -> rmcp::model::Tool {
        let schema = tool.parameters_schema().unwrap_or_else(|| {
            serde_json::json!({ "type": "object", "properties": {} })
        });
        let input_schema: rmcp::model::JsonObject = match schema {
            Value::Object(map) => map.into_iter().collect(),
            _ => Default::default(),
        };
        rmcp::model::Tool::new(
            tool.name().to_string(),
            tool.description().to_string(),
            Arc::new(input_schema),
        )
    }
}

impl ServerHandler for ZavoraMcpServer {
    fn get_info(&self) -> ServerInfo {
        ServerInfo::new(ServerCapabilities::builder().enable_tools().build())
            .with_server_info(Implementation::new("zavora-cli", env!("CARGO_PKG_VERSION")))
            .with_instructions(
                "Zavora CLI agent tools: file read/write/edit, bash execution, \
                 glob/grep search, github ops, and more.",
            )
    }

    fn list_tools(
        &self,
        _request: Option<PaginatedRequestParams>,
        _context: rmcp::service::RequestContext<rmcp::RoleServer>,
    ) -> impl std::future::Future<Output = Result<ListToolsResult, McpError>> + Send + '_ {
        let tools: Vec<rmcp::model::Tool> = self
            .tools
            .iter()
            .map(|t| self.adk_tool_to_mcp(t.as_ref()))
            .collect();
        std::future::ready(Ok(ListToolsResult::with_all_items(tools)))
    }

    fn call_tool(
        &self,
        request: CallToolRequestParams,
        _context: rmcp::service::RequestContext<rmcp::RoleServer>,
    ) -> impl std::future::Future<Output = Result<CallToolResult, McpError>> + Send + '_ {
        let tool_name = request.name.to_string();
        let args: Value = request
            .arguments
            .map(|obj| Value::Object(obj.into_iter().collect()))
            .unwrap_or(Value::Object(Default::default()));

        async move {
            let tool = self
                .tools
                .iter()
                .find(|t| t.name() == tool_name)
                .ok_or_else(|| {
                    McpError::invalid_params(format!("tool '{}' not found", tool_name), None)
                })?;

            let ctx: Arc<dyn adk_rust::ToolContext> = Arc::new(
                adk_tool::SimpleToolContext::new("mcp-server"),
            );

            match tool.execute(ctx, args).await {
                Ok(result) => {
                    let text = if result.is_string() {
                        result.as_str().unwrap_or("").to_string()
                    } else {
                        serde_json::to_string_pretty(&result).unwrap_or_default()
                    };
                    Ok(CallToolResult::success(vec![Content::text(text)]))
                }
                Err(e) => Ok(CallToolResult::error(vec![Content::text(e.to_string())])),
            }
        }
    }
}

/// Run the MCP server on stdio.
pub async fn run_mcp_server() -> anyhow::Result<()> {
    let server = ZavoraMcpServer::new();
    let service = server.serve(stdio()).await.map_err(|e| {
        anyhow::anyhow!("MCP server error: {:?}", e)
    })?;
    service.waiting().await?;
    Ok(())
}
