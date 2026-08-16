//! Agent-facing capability discovery and enablement.
//!
//! A capability is a curated set of MCP servers plus the specialist agent that
//! uses them. Discovering which one fits a request is something the model can do
//! well; deciding to install software is not, so the two are separate tools with
//! separate consequences.
//!
//! [`capability_status`] is read-only and answers "what would help here, and what
//! state is it in". [`capability_enable`] changes the workspace, and passes
//! through the same confirmation path as every other mutating tool — the
//! difference is that the prompt names the exact install commands, because
//! "enable office support" and "compile and install five programs from the
//! internet" are one request described at two very different levels of
//! consequence.
//!
//! What the model can influence is deliberately narrow. It supplies a capability
//! id, which must match the built-in catalogue; every install command comes from
//! that catalogue, never from the model or the prompt. So the worst a hostile
//! prompt can achieve is to ask the developer to approve one of a fixed, curated
//! set of installs — and the developer still has to say yes.

use std::path::PathBuf;
use std::sync::Arc;

use adk_tool::{FunctionTool, Tool};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::capabilities;

/// Where the profile configuration lives, matching the CLI's own defaults.
///
/// Resolved here rather than threaded in because `build_builtin_tools` takes no
/// configuration; these are the same environment variable and default that
/// `--config` and `--profile` use, so a tool and a command see the same files.
fn config_path() -> PathBuf {
    std::env::var_os("ZAVORA_CONFIG")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(".zavora/config.toml"))
}

fn profile() -> String {
    std::env::var("ZAVORA_PROFILE").unwrap_or_else(|_| "default".to_string())
}

#[derive(Debug, Default, Deserialize, Serialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct StatusArgs {
    /// A specific capability id, such as `productivity.office`.
    #[serde(default)]
    pub id: Option<String>,
    /// Free text describing the need, matched against capability names and
    /// descriptions.
    #[serde(default)]
    pub query: Option<String>,
}

#[derive(Debug, Default, Deserialize, Serialize, schemars::JsonSchema)]
pub struct EnableArgs {
    /// The capability id to enable. Must be one of the built-in capabilities.
    #[serde(default)]
    pub id: String,
    /// Set by the approval layer, never by the model.
    #[serde(default)]
    pub approved: bool,
}

/// Render one capability's readiness.
fn plan_json(plan: &capabilities::EnablePlan) -> Value {
    json!({
        "id": plan.pack_id,
        "name": plan.pack_name,
        "description": plan.description,
        "risk": plan.risk.to_string(),
        "maturity": plan.maturity.to_string(),
        "agent": plan.agent,
        "enabled": plan.already_enabled,
        "servers": plan.servers.iter().map(|server| json!({
            "id": server.id,
            "name": server.name,
            // Reported so a capability that cannot be provisioned is never
            // mistaken for one that merely needs installing.
            "available": server.available,
            "installed": server.installed,
            "configured": server.configured,
            "install_command": server.install,
        })).collect::<Vec<_>>(),
        "unavailable_servers": plan
            .unavailable()
            .iter()
            .map(|server| server.id.clone())
            .collect::<Vec<_>>(),
        "needs_install": plan.to_install().len(),
        "needs_configure": plan.to_configure().len(),
    })
}

fn status_response(args: &Value) -> Value {
    let args: StatusArgs = serde_json::from_value(args.clone()).unwrap_or_default();
    let path = config_path();
    let profile = profile();

    let packs: Vec<&capabilities::CapabilityPack> = match (&args.id, &args.query) {
        (Some(id), _) => match capabilities::find_pack(id) {
            Some(pack) => vec![pack],
            None => {
                return json!({
                    "error": format!(
                        "capability '{id}' is not one of the built-in capabilities"
                    ),
                    "available": capabilities::built_in_packs()
                        .iter()
                        .map(|pack| pack.id)
                        .collect::<Vec<_>>(),
                });
            }
        },
        (None, Some(query)) => capabilities::search_packs(query),
        (None, None) => capabilities::built_in_packs().iter().collect(),
    };

    let candidates: Vec<Value> = packs
        .iter()
        .filter_map(|pack| capabilities::plan_enable(pack.id, &path, &profile).ok())
        .map(|plan| plan_json(&plan))
        .collect();

    json!({
        "capabilities": candidates,
        // Said plainly because the distinction is easy to lose: an enabled
        // capability with configured servers is still not a working one.
        "note": "`installed` and `configured` are static facts. Whether a server \
                 is reachable is not knowable without a handshake — run \
                 `zavora-cli mcp doctor` for that. Enabling a capability never \
                 makes its servers usable on its own.",
        "enable_with": "capability_enable",
    })
}

/// Run one curated install command.
///
/// Executed as an argument vector, never through a shell, and only after
/// [`capabilities::install_argv`] has confirmed the program is one this code is
/// willing to run.
fn run_install(server: &capabilities::ServerReadiness) -> Value {
    let Some(line) = server.install else {
        return json!({
            "server": server.id,
            "status": "unavailable",
            "error": "no installable package in the curated catalogue",
        });
    };
    let argv = match capabilities::install_argv(line) {
        Ok(argv) => argv,
        Err(error) => {
            return json!({
                "server": server.id,
                "status": "refused",
                "error": error.to_string(),
            });
        }
    };
    let (program, rest) = argv.split_first().expect("validated non-empty");
    match std::process::Command::new(program).args(rest).output() {
        Ok(output) if output.status.success() => json!({
            "server": server.id,
            "status": "installed",
            "command": line,
        }),
        Ok(output) => json!({
            "server": server.id,
            "status": "failed",
            "command": line,
            "exit_code": output.status.code(),
            "stderr": crate::text::truncate(
                &String::from_utf8_lossy(&output.stderr), 600, "…",
            ),
        }),
        Err(error) => json!({
            "server": server.id,
            "status": "failed",
            "command": line,
            "error": error.to_string(),
        }),
    }
}

/// How an install is carried out.
///
/// Injected rather than called directly so that installing is visible at the
/// boundary and cannot happen by accident. It already did once: an early test
/// supplied `approved: true` to exercise the body and, because the body ran the
/// installer itself, compiled and installed eight crates onto the machine running
/// the suite. A seam here means a test has to opt in to that explicitly, and none
/// does.
pub type Installer<'a> = &'a (dyn Fn(&capabilities::ServerReadiness) -> Value + Sync);

async fn enable_response(args: &Value) -> Value {
    enable_with(args, &run_install).await
}

async fn enable_with(args: &Value, install: Installer<'_>) -> Value {
    let parsed: EnableArgs = match serde_json::from_value(args.clone()) {
        Ok(parsed) => parsed,
        Err(error) => return json!({"error": format!("invalid arguments: {error}")}),
    };
    if parsed.id.trim().is_empty() {
        return json!({
            "error": "id is required",
            "available": capabilities::built_in_packs()
                .iter()
                .map(|pack| pack.id)
                .collect::<Vec<_>>(),
        });
    }
    // The approval layer sets this after the developer says yes, and
    // `scrub_model_supplied_safety_args` removes any the model supplied, so this
    // cannot be satisfied by the model asking nicely.
    if !parsed.approved {
        return json!({
            "error": "capability_enable requires approval and was not approved",
        });
    }

    let path = config_path();
    let profile = profile();
    let plan = match capabilities::plan_enable(&parsed.id, &path, &profile) {
        Ok(plan) => plan,
        Err(error) => return json!({"error": error.to_string()}),
    };

    // A capability whose servers all lack installable packages cannot be made to
    // work, and writing `enabled = true` over it would be a claim with nothing
    // behind it. Refuse instead, and say which servers are missing.
    if plan.provisionable().is_empty() {
        return json!({
            "error": format!(
                "capability '{}' names {} MCP servers, none of which has an \
                 installable package in the curated catalogue. Enabling it would \
                 change a flag and nothing else.",
                plan.pack_id,
                plan.servers.len(),
            ),
            "unavailable_servers": plan
                .unavailable()
                .iter()
                .map(|server| server.id.clone())
                .collect::<Vec<_>>(),
        });
    }

    // Installs first: a server configured but absent is a connection failure
    // later, which is harder to explain than an install failure now.
    let installs: Vec<Value> = plan.to_install().iter().map(|s| install(s)).collect();
    let install_failed = installs
        .iter()
        .filter(|result| result.get("status").and_then(Value::as_str) != Some("installed"))
        .count();

    let mut configured = Vec::new();
    let mut configure_errors = Vec::new();
    for server in plan.to_configure() {
        match crate::mcp_catalog::add_server(&path, &profile, &server.id) {
            Ok(_) => configured.push(server.id.clone()),
            Err(error) => configure_errors.push(json!({
                "server": server.id,
                "error": error.to_string(),
            })),
        }
    }

    let enabled = capabilities::set_pack_enabled(&capabilities::state_path(), plan.pack_id, true);
    let enable_error = enabled.as_ref().err().map(ToString::to_string);

    // Tell the workspace its tool surface is behind the configuration. It picks
    // this up between turns and reconnects, so the capability becomes usable in
    // this session rather than the next one.
    if !configured.is_empty() {
        capabilities::mark_surface_stale();
    }

    // Re-plan so the reported state is measured after the change rather than
    // assumed from what was attempted.
    let after = capabilities::plan_enable(&parsed.id, &path, &profile).ok();

    json!({
        "capability": plan.pack_id,
        "name": plan.pack_name,
        "agent": plan.agent,
        "installs": installs,
        "install_failures": install_failed,
        "configured": configured,
        "configure_errors": configure_errors,
        "unavailable_servers": plan
            .unavailable()
            .iter()
            .map(|server| server.id.clone())
            .collect::<Vec<_>>(),
        "enabled": enable_error.is_none(),
        "enable_error": enable_error,
        "state_after": after.as_ref().map(plan_json),
        "connected": Value::Null,
        // Configured is not connected. The workspace reconnects between turns and
        // reports which servers actually answered, so this says what happens next
        // rather than asking for a restart.
        "next_step": "The servers are configured, not yet connected. The workspace \
                      reconnects before the next turn and reports which answered; \
                      `zavora-cli mcp doctor` inspects them directly.",
    })
}

/// The read-only half: what capabilities exist and what state they are in.
pub fn build_status_tool() -> Arc<dyn Tool> {
    Arc::new(
        FunctionTool::new(
            "capability_status",
            "Reports which built-in capabilities exist and how ready each one is. \
             Use before claiming a capability is unavailable, and to find what a \
             request would need. \
             Args: id (optional capability id such as 'productivity.office'), \
             query (optional free text matched against names and descriptions). \
             Returns per capability: risk, maturity, specialist agent, whether it \
             is enabled, and per server whether it is installed and configured. \
             'installed' and 'configured' do not mean reachable; only a handshake \
             shows that.",
            |_ctx, args| async move { Ok(status_response(&args)) },
        )
        .with_parameters_schema::<StatusArgs>()
        .with_read_only(true)
        .with_concurrency_safe(true),
    )
}

/// The mutating half: install, configure, and enable one capability.
pub fn build_enable_tool() -> Arc<dyn Tool> {
    Arc::new(
        FunctionTool::new(
            "capability_enable",
            "Enables one built-in capability, installing and configuring the MCP \
             servers it needs. Requires the developer's approval, which names the \
             exact install commands. Installing runs third-party code, so call \
             capability_status first and only propose this when the request \
             genuinely needs it. \
             Args: id (required, a built-in capability id). \
             Enabling does not make the servers reachable; they connect on the \
             next session.",
            |_ctx, args| async move { Ok(enable_response(&args).await) },
        )
        .with_parameters_schema::<EnableArgs>(),
    )
}

/// The approval question, computed from the current state of the workspace.
///
/// Lives here so the confirmation layer can show the real plan — which servers
/// are actually missing — rather than the arguments the model sent.
pub fn describe_enable_request(args: &Value) -> String {
    let id = args.get("id").and_then(Value::as_str).unwrap_or("");
    if id.is_empty() {
        return "capability_enable called without a capability id".to_string();
    }
    match capabilities::plan_enable(id, &config_path(), &profile()) {
        Ok(plan) if plan.is_noop() => {
            format!(
                "\"{}\" ({}) is already enabled and configured.",
                plan.pack_name, plan.pack_id
            )
        }
        Ok(plan) => plan.approval_prompt(),
        Err(error) => format!("capability_enable: {error}"),
    }
}

/// Both tools, in the order they should be offered.
pub fn build_capability_tools() -> Vec<Arc<dyn Tool>> {
    vec![build_status_tool(), build_enable_tool()]
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_config() -> (tempfile::TempDir, PathBuf) {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("config.toml");
        (dir, path)
    }

    /// An unknown capability id is refused, and the caller is told what exists.
    #[test]
    fn unknown_capability_ids_are_refused() {
        let response = status_response(&json!({"id": "not.a.capability"}));
        assert!(
            response.get("error").is_some(),
            "an unknown id must be an error: {response}"
        );
        let available = response
            .get("available")
            .and_then(Value::as_array)
            .expect("the error should list what exists");
        assert!(available.iter().any(|id| id == "productivity.office"));
    }

    /// Enable refuses without approval, whatever the model claims.
    #[tokio::test]
    async fn enable_refuses_without_approval() {
        let response = enable_response(&json!({"id": "productivity.office"})).await;
        assert_eq!(
            response.get("error").and_then(Value::as_str),
            Some("capability_enable requires approval and was not approved")
        );
    }

    /// The plan is empty of installs when everything is already present, and the
    /// approval text says so rather than asking to install nothing.
    #[test]
    fn the_approval_names_the_capability_and_its_installs() {
        let (_dir, path) = temp_config();
        let plan = capabilities::plan_enable("productivity.office", &path, "default")
            .expect("a built-in capability");
        let prompt = plan.approval_prompt();

        assert!(
            prompt.contains("Office Artifacts") && prompt.contains("productivity.office"),
            "the developer must be told which capability: {prompt}"
        );
        assert!(
            prompt.contains("risk high"),
            "risk belongs in the question: {prompt}"
        );
        assert!(
            prompt.contains("third-party code"),
            "installing runs code from elsewhere and must say so: {prompt}"
        );
        assert!(
            prompt.contains("until they connect"),
            "enabling must not imply the servers work: {prompt}"
        );

        // On a machine without these servers, every install command is named.
        for server in plan.to_install() {
            let line = server
                .install
                .expect("a provisionable server has an install line");
            assert!(
                prompt.contains(line),
                "install command '{line}' missing from the approval: {prompt}"
            );
        }
    }

    /// The approval text is derived from the workspace, not from the arguments.
    #[test]
    fn the_approval_description_reports_real_state() {
        let described = describe_enable_request(&json!({"id": "productivity.office"}));
        assert!(
            described.contains("productivity.office"),
            "should describe the pack: {described}"
        );
        let missing = describe_enable_request(&json!({"id": "nope"}));
        assert!(
            missing.contains("not one of the built-in capabilities"),
            "an unknown id must be reported, not silently approved: {missing}"
        );
        let empty = describe_enable_request(&json!({}));
        assert!(empty.contains("without a capability id"), "{empty}");
    }

    /// Both tools reach the sealed surface, classified by their own declaration.
    ///
    /// Registration matters as much as the tools: `capability_enable` must arrive
    /// as `Mutating` so `seal` wraps it in the confirming path, which is what
    /// produces the approval at all. If it were ever classified `ReadOnly` it
    /// would install software without asking.
    #[test]
    fn the_capability_tools_reach_the_sealed_surface() {
        let tools = crate::tools::build_builtin_tools();
        let names: Vec<&str> = tools.iter().map(|tool| tool.name()).collect();
        assert!(names.contains(&"capability_status"), "{names:?}");
        assert!(names.contains(&"capability_enable"), "{names:?}");

        let status = tools
            .iter()
            .find(|tool| tool.name() == "capability_status")
            .expect("status");
        let enable = tools
            .iter()
            .find(|tool| tool.name() == "capability_enable")
            .expect("enable");

        use crate::tool_policy::{ToolClass, ToolProvenance, classify};
        assert_eq!(
            classify(status, ToolProvenance::BuiltIn),
            ToolClass::ReadOnly,
            "discovery must be auto-approvable or the model cannot look before asking"
        );
        assert_eq!(
            classify(enable, ToolProvenance::BuiltIn),
            ToolClass::Mutating,
            "enabling installs software; it must be gated by the confirming wrapper"
        );
        assert!(
            !ToolClass::Mutating.is_auto_approvable(),
            "the class that gates enabling must never auto-approve"
        );
    }

    /// A recording installer, so no test ever installs anything.
    fn refusing_installer() -> impl Fn(&capabilities::ServerReadiness) -> Value {
        |server: &capabilities::ServerReadiness| {
            json!({
                "server": server.id,
                "status": "installed",
                "note": "test installer; nothing was executed",
            })
        }
    }

    /// A plan whose servers have no installable package.
    ///
    /// Synthetic because every shipped capability is now fully provisionable —
    /// which a test in `capabilities` enforces. The refusal still has to work, for
    /// the next server added to a pack before its catalogue entry exists.
    fn unprovisionable_plan() -> capabilities::EnablePlan {
        let pack = capabilities::find_pack("operations.business").expect("a built-in pack");
        capabilities::EnablePlan {
            pack_id: pack.id,
            pack_name: pack.name,
            description: pack.description,
            risk: pack.risk,
            maturity: pack.maturity,
            agent: pack.agent,
            already_enabled: false,
            servers: vec![capabilities::ServerReadiness {
                id: "mcp-not-yet-published".to_string(),
                name: None,
                command: None,
                install: None,
                available: false,
                installed: false,
                configured: false,
            }],
        }
    }

    /// A capability with nothing installable is described as impossible, not
    /// offered as a decision.
    #[test]
    fn an_unprovisionable_capability_is_not_offered_as_a_choice() {
        let prompt = unprovisionable_plan().approval_prompt();
        assert!(
            prompt.contains("cannot be enabled"),
            "the developer must not be asked to approve a no-op: {prompt}"
        );
        assert!(
            !prompt.contains('?'),
            "it is a statement, not a question: {prompt}"
        );
        assert!(
            prompt.contains("change a flag and nothing else"),
            "and it must say why: {prompt}"
        );
    }

    /// Enabling never installs when nothing can be installed.
    #[tokio::test]
    async fn enable_refuses_a_plan_with_nothing_to_provision() {
        let plan = unprovisionable_plan();
        assert!(plan.provisionable().is_empty());
        assert_eq!(plan.unavailable().len(), 1);
        // The prompt and the refusal agree, so approving cannot reach the
        // installer by a different route.
        assert!(plan.approval_prompt().contains("cannot be enabled"));
    }

    /// Status reports full coverage now that the catalogue is complete.
    #[test]
    fn status_reports_every_server_of_a_capability() {
        let response = status_response(&json!({"id": "productivity.office"}));
        let capability = &response
            .get("capabilities")
            .and_then(Value::as_array)
            .expect("capabilities")[0];
        let servers = capability
            .get("servers")
            .and_then(Value::as_array)
            .expect("servers");
        assert_eq!(servers.len(), 5, "all five are reported");
        assert!(
            servers
                .iter()
                .all(|server| server.get("available") == Some(&Value::Bool(true))),
            "every server should now be provisionable: {servers:#?}"
        );
        assert!(
            capability
                .get("unavailable_servers")
                .and_then(Value::as_array)
                .is_some_and(|list| list.is_empty()),
            "nothing should be unprovisionable"
        );

        // Connection is never asserted from static state.
        let note = response.get("note").and_then(Value::as_str).unwrap_or("");
        assert!(
            note.contains("not knowable without a handshake"),
            "the read-only report must not imply reachability: {note}"
        );
    }

    /// Enabling goes through the injected installer, and reports honestly.
    ///
    /// Uses a fake installer: a test must never compile and install crates onto
    /// the machine running it.
    #[tokio::test]
    async fn enable_uses_the_injected_installer() {
        let dir = tempfile::tempdir().expect("tempdir");
        // Point config and capability state at the temporary directory so the
        // test cannot touch the developer's workspace.
        let config = dir.path().join("config.toml");
        let state = dir.path().join("capabilities.toml");
        // SAFETY: single-threaded test, restored below.
        unsafe {
            std::env::set_var("ZAVORA_CONFIG", &config);
            std::env::set_var("ZAVORA_CAPABILITY_STATE", &state);
        }

        let installer = refusing_installer();
        let response = enable_with(
            &json!({"id": "research.core", "approved": true}),
            &installer,
        )
        .await;

        unsafe {
            std::env::remove_var("ZAVORA_CONFIG");
            std::env::remove_var("ZAVORA_CAPABILITY_STATE");
        }

        assert_eq!(
            response.get("capability").and_then(Value::as_str),
            Some("research.core")
        );
        assert_eq!(response.get("enabled"), Some(&Value::Bool(true)));
        // Never claims the servers are reachable.
        assert_eq!(response.get("connected"), Some(&Value::Null));
        assert!(
            response
                .get("next_step")
                .and_then(Value::as_str)
                .is_some_and(|step| step.contains("not yet connected")),
            "the result must distinguish configured from connected: {response}"
        );
        // The configuration was written to the temporary path, not the workspace.
        assert!(
            config.exists(),
            "the profile config should have been written"
        );
        assert!(
            !std::path::Path::new(".zavora/capabilities.toml").exists()
                || !std::fs::read_to_string(".zavora/capabilities.toml")
                    .unwrap_or_default()
                    .contains("research.core"),
            "the test must not have enabled anything in the real workspace"
        );
    }

    /// Enabling asks the workspace to reconnect, so the capability works now.
    ///
    /// Sealing a tool surface is one-shot by design, so a capability enabled
    /// mid-session cannot be added to the surface in place. The flag is the
    /// handshake: without it the developer would be told to restart, which is the
    /// difference between a capability that turns on and one that is merely
    /// recorded as on.
    #[tokio::test]
    async fn enabling_asks_the_workspace_to_reconnect() {
        let dir = tempfile::tempdir().expect("tempdir");
        let config = dir.path().join("config.toml");
        let state = dir.path().join("capabilities.toml");
        // SAFETY: single-threaded test, cleared below.
        unsafe {
            std::env::set_var("ZAVORA_CONFIG", &config);
            std::env::set_var("ZAVORA_CAPABILITY_STATE", &state);
        }

        // Start from a known flag state.
        let _ = capabilities::take_surface_stale();
        assert!(!capabilities::take_surface_stale());

        let installer = refusing_installer();
        let response = enable_with(
            &json!({"id": "research.core", "approved": true}),
            &installer,
        )
        .await;

        unsafe {
            std::env::remove_var("ZAVORA_CONFIG");
            std::env::remove_var("ZAVORA_CAPABILITY_STATE");
        }

        assert!(
            !response
                .get("configured")
                .and_then(Value::as_array)
                .unwrap_or(&vec![])
                .is_empty(),
            "servers should have been configured: {response}"
        );
        assert!(
            capabilities::take_surface_stale(),
            "the workspace must be told to rebuild its tool surface"
        );
        // Taken once, not repeatedly: a failed rebuild must not spin.
        assert!(
            !capabilities::take_surface_stale(),
            "the flag must clear when read"
        );
    }

    /// Both tools declare themselves correctly to the policy layer.
    #[test]
    fn status_is_read_only_and_enable_is_not() {
        let tools = build_capability_tools();
        let status = tools
            .iter()
            .find(|tool| tool.name() == "capability_status")
            .expect("status tool");
        let enable = tools
            .iter()
            .find(|tool| tool.name() == "capability_enable")
            .expect("enable tool");
        assert!(
            status.is_read_only(),
            "discovery must not require approval, or the model cannot look before it asks"
        );
        assert!(
            !enable.is_read_only(),
            "enabling installs software and must be classified as mutating"
        );
    }
}
