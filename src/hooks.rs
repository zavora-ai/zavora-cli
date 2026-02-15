use std::collections::HashMap;
use std::fmt;
use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::tool_policy::matches_wildcard;

// ---------------------------------------------------------------------------
// Hook lifecycle points
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum HookPoint {
    AgentSpawn,
    PromptSubmit,
    PreTool,
    PostTool,
    Stop,
}

impl fmt::Display for HookPoint {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            HookPoint::AgentSpawn => write!(f, "agent_spawn"),
            HookPoint::PromptSubmit => write!(f, "prompt_submit"),
            HookPoint::PreTool => write!(f, "pre_tool"),
            HookPoint::PostTool => write!(f, "post_tool"),
            HookPoint::Stop => write!(f, "stop"),
        }
    }
}

// ---------------------------------------------------------------------------
// Hook configuration (lives in agent catalog TOML)
// ---------------------------------------------------------------------------

const DEFAULT_TIMEOUT_MS: u64 = 30_000;
const DEFAULT_MAX_OUTPUT: usize = 10_240;

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq, Hash)]
#[serde(deny_unknown_fields)]
pub struct HookConfig {
    pub command: String,
    #[serde(default = "default_timeout_ms")]
    pub timeout_ms: u64,
    #[serde(default = "default_max_output")]
    pub max_output: usize,
    /// Glob matcher for pre_tool / post_tool scoping (e.g. "fs_*", "execute_bash")
    #[serde(skip_serializing_if = "Option::is_none")]
    pub matcher: Option<String>,
}

fn default_timeout_ms() -> u64 {
    DEFAULT_TIMEOUT_MS
}
fn default_max_output() -> usize {
    DEFAULT_MAX_OUTPUT
}

// ---------------------------------------------------------------------------
// Hook execution result
// ---------------------------------------------------------------------------

/// Exit code 2 from a pre_tool hook means "block this tool call".
pub const HOOK_EXIT_BLOCK: i32 = 2;

#[derive(Debug, Clone)]
pub struct HookResult {
    pub hook_point: HookPoint,
    pub command: String,
    pub exit_code: i32,
    pub output: String,
    pub duration: Duration,
}

impl HookResult {
    pub fn is_block(&self) -> bool {
        self.hook_point == HookPoint::PreTool && self.exit_code == HOOK_EXIT_BLOCK
    }
}

// ---------------------------------------------------------------------------
// Tool context passed to pre_tool / post_tool hooks via stdin JSON
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize)]
pub struct HookToolContext {
    pub tool_name: String,
    pub tool_input: Value,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tool_response: Option<Value>,
}

// ---------------------------------------------------------------------------
// Hook executor
// ---------------------------------------------------------------------------

#[derive(Debug, Default)]
pub struct HookExecutor {
    hooks: HashMap<HookPoint, Vec<HookConfig>>,
}

impl HookExecutor {
    pub fn new(hooks: HashMap<HookPoint, Vec<HookConfig>>) -> Self {
        Self { hooks }
    }

    pub fn is_empty(&self) -> bool {
        self.hooks.values().all(|v| v.is_empty())
    }

    /// Run all hooks registered for the given point. Returns results in order.
    /// For pre_tool / post_tool, only hooks whose matcher matches the tool name fire.
    pub async fn run(
        &self,
        point: HookPoint,
        prompt: Option<&str>,
        tool_ctx: Option<&HookToolContext>,
    ) -> Vec<HookResult> {
        let Some(hooks) = self.hooks.get(&point) else {
            return Vec::new();
        };

        let mut results = Vec::new();
        for hook in hooks {
            if !hook_matches_tool(hook, tool_ctx) {
                continue;
            }
            let result = run_single_hook(point, hook, prompt, tool_ctx).await;
            tracing::info!(
                hook_point = %point,
                command = %hook.command,
                exit_code = result.exit_code,
                duration_ms = result.duration.as_millis() as u64,
                blocked = result.is_block(),
                "Hook executed"
            );
            results.push(result);
        }
        results
    }

    /// Convenience: run pre_tool hooks and return true if any hook blocks.
    pub async fn run_pre_tool(&self, tool_ctx: &HookToolContext) -> (bool, Vec<HookResult>) {
        let results = self.run(HookPoint::PreTool, None, Some(tool_ctx)).await;
        let blocked = results.iter().any(|r| r.is_block());
        (blocked, results)
    }
}

// ---------------------------------------------------------------------------
// Matcher logic
// ---------------------------------------------------------------------------

fn hook_matches_tool(hook: &HookConfig, tool_ctx: Option<&HookToolContext>) -> bool {
    let Some(pattern) = hook.matcher.as_deref() else {
        return true; // no matcher → fires for all
    };
    let Some(ctx) = tool_ctx else {
        return true; // non-tool hook points don't filter
    };
    matches_wildcard(pattern, &ctx.tool_name)
}

// ---------------------------------------------------------------------------
// Single hook execution
// ---------------------------------------------------------------------------

async fn run_single_hook(
    point: HookPoint,
    hook: &HookConfig,
    prompt: Option<&str>,
    tool_ctx: Option<&HookToolContext>,
) -> HookResult {
    let start = Instant::now();
    let timeout = Duration::from_millis(hook.timeout_ms);

    let mut input = json!({ "hook_point": point.to_string() });
    if let Some(p) = prompt {
        input["prompt"] = Value::String(p.to_string());
    }
    if let Some(ctx) = tool_ctx {
        input["tool_name"] = Value::String(ctx.tool_name.clone());
        input["tool_input"] = ctx.tool_input.clone();
        if let Some(resp) = &ctx.tool_response {
            input["tool_response"] = resp.clone();
        }
    }
    let json_input = serde_json::to_string(&input).unwrap_or_default();

    let command_future = async {
        let mut cmd = tokio::process::Command::new("bash");
        cmd.arg("-c")
            .arg(&hook.command)
            .stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped());

        let mut child = cmd.spawn()?;
        if let Some(mut stdin) = child.stdin.take() {
            use tokio::io::AsyncWriteExt;
            let _ = stdin.write_all(json_input.as_bytes()).await;
            let _ = stdin.shutdown().await;
        }
        child.wait_with_output().await
    };

    match tokio::time::timeout(timeout, command_future).await {
        Ok(Ok(output)) => {
            let exit_code = output.status.code().unwrap_or(-1);
            let raw = if exit_code == 0 {
                String::from_utf8_lossy(&output.stdout)
            } else {
                String::from_utf8_lossy(&output.stderr)
            };
            let truncated = if raw.len() > hook.max_output {
                format!("{}... truncated", &raw[..hook.max_output])
            } else {
                raw.to_string()
            };
            HookResult {
                hook_point: point,
                command: hook.command.clone(),
                exit_code,
                output: truncated,
                duration: start.elapsed(),
            }
        }
        Ok(Err(err)) => HookResult {
            hook_point: point,
            command: hook.command.clone(),
            exit_code: -1,
            output: format!("failed to execute: {err}"),
            duration: start.elapsed(),
        },
        Err(_) => HookResult {
            hook_point: point,
            command: hook.command.clone(),
            exit_code: -1,
            output: format!("timed out after {}ms", hook.timeout_ms),
            duration: start.elapsed(),
        },
    }
}

// ---------------------------------------------------------------------------
// Parse hooks from agent catalog TOML structure
// ---------------------------------------------------------------------------

/// Parse a hooks table from agent config. Expected TOML shape:
/// ```toml
/// [agents.coder.hooks.pre_tool]
/// command = "echo checking"
/// matcher = "fs_*"
/// ```
/// or as arrays:
/// ```toml
/// [[agents.coder.hooks.pre_tool]]
/// command = "echo first"
/// [[agents.coder.hooks.pre_tool]]
/// command = "echo second"
/// ```
pub fn parse_hooks_map(
    raw: &HashMap<String, Vec<HookConfig>>,
) -> HashMap<HookPoint, Vec<HookConfig>> {
    let mut map = HashMap::new();
    for (key, hooks) in raw {
        let point = match key.as_str() {
            "agent_spawn" => HookPoint::AgentSpawn,
            "prompt_submit" => HookPoint::PromptSubmit,
            "pre_tool" => HookPoint::PreTool,
            "post_tool" => HookPoint::PostTool,
            "stop" => HookPoint::Stop,
            other => {
                tracing::warn!(hook_point = other, "Unknown hook point; skipping");
                continue;
            }
        };
        map.insert(point, hooks.clone());
    }
    map
}
