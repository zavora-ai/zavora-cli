/// Sandboxed code execution via adk-sandbox (feature-gated: `sandbox`).
use adk_rust::prelude::*;
use std::sync::Arc;

/// Build the sandbox code execution tool.
pub fn build_sandbox_tool() -> Arc<dyn Tool> {
    let backend = Arc::new(adk_sandbox::ProcessBackend::default());
    Arc::new(adk_sandbox::SandboxTool::new(backend))
}
