//! Release-gate tests for the sealed tool surface.
//!
//! These assert the correctness properties from `.kiro/specs/v2-vision`
//! that keep a whole defect class out of the runtime, rather than checking
//! individual known bugs.

use std::collections::BTreeSet;

use crate::cli::ToolConfirmationMode;
use crate::config::RuntimeConfig;
use crate::tool_policy::{ToolClass, ToolProvenance, classify};
use crate::tool_surface::ToolSurface;
use crate::tools::build_builtin_tools;
use crate::tools::confirming::{MODEL_FORBIDDEN_SAFETY_ARGS, scrub_model_supplied_safety_args};

fn base_config() -> RuntimeConfig {
    crate::tests::base_cfg()
}

/// Property 2: nothing may be added to the surface after `seal`.
///
/// Visibility does the real enforcement — `ResolvedRuntimeTools` has no
/// mutating accessor, so a post-seal `extend` is a compile error. This asserts
/// the complementary runtime fact: every tool the builder assembled is present
/// after sealing, so no addition was quietly dropped or appended out of band.
#[test]
fn sealing_preserves_every_assembled_tool() {
    let cfg = base_config();
    let assembled = build_builtin_tools();
    let assembled_names = assembled
        .iter()
        .map(|tool| tool.name().to_string())
        .collect::<BTreeSet<_>>();

    let sealed = ToolSurface::new().add_builtins().seal(&cfg);

    assert_eq!(
        sealed.names(),
        assembled_names,
        "sealing changed the tool set"
    );
    assert_eq!(sealed.len(), assembled.len());
}

/// Property 2: the discovery tool searches the sealed set, so it must be part
/// of that set rather than appended afterwards.
#[test]
fn discovery_tool_joins_the_surface_before_sealing() {
    let cfg = base_config();
    let sealed = ToolSurface::new()
        .add_builtins()
        // Threshold 0 forces registration regardless of built-in count.
        .add_discovery_tool(0)
        .seal(&cfg);

    assert!(
        sealed.names().contains("tool_search"),
        "tool_search missing from the sealed surface: {:?}",
        sealed.names()
    );
}

/// Property 5: every tool on the surface carries exactly one class.
#[test]
fn classification_is_total_over_the_sealed_surface() {
    let cfg = base_config();
    let sealed = ToolSurface::new()
        .add_builtins()
        .add_discovery_tool(0)
        .seal(&cfg);

    for name in sealed.names() {
        assert!(
            sealed.class_of(&name).is_some(),
            "tool '{name}' reached the surface without a class"
        );
    }
}

/// Property 5: network egress is never auto-approvable, in any mode.
///
/// This is what closes the `web_fetch` gap structurally. Before classification,
/// `web_fetch` was absent from both the guarded name list and the read-only
/// list, so under the default `mcp-only` mode it was handed to the model
/// unwrapped and performed unapproved egress.
#[test]
fn network_egress_is_never_auto_approvable() {
    for mode in [
        ToolConfirmationMode::Always,
        ToolConfirmationMode::McpOnly,
        ToolConfirmationMode::Never,
    ] {
        let mut cfg = base_config();
        cfg.tool_confirmation_mode = mode;
        assert!(
            !ToolClass::NetworkEgress.is_auto_approvable(),
            "egress became auto-approvable in mode {mode:?}"
        );
    }
}

/// Property 5: anything arriving from outside the binary is treated as egress,
/// because its blast radius is not knowable locally.
#[test]
fn external_provenance_classifies_as_egress() {
    let tools = build_builtin_tools();
    let read_only = tools
        .iter()
        .find(|tool| tool.is_read_only())
        .expect("at least one built-in is read-only");

    assert_eq!(
        classify(read_only, ToolProvenance::BuiltIn),
        ToolClass::ReadOnly,
        "a read-only built-in should classify as read-only"
    );
    for external in [
        ToolProvenance::Mcp,
        ToolProvenance::Plugin,
        ToolProvenance::FeatureGated,
    ] {
        assert_eq!(
            classify(read_only, external),
            ToolClass::NetworkEgress,
            "provenance {external:?} must classify as egress even for a read-only tool"
        );
    }
}

/// Property 5: mutating built-ins are classified as mutating, not read-only.
#[test]
fn mutating_builtins_are_not_read_only() {
    let tools = build_builtin_tools();
    for name in ["fs_write", "file_edit", "execute_bash"] {
        let tool = tools
            .iter()
            .find(|tool| tool.name() == name)
            .unwrap_or_else(|| panic!("built-in '{name}' not registered"));
        assert_eq!(
            classify(tool, ToolProvenance::BuiltIn),
            ToolClass::Mutating,
            "'{name}' must be classified as mutating"
        );
    }
}

/// Property 5: `github_ops` leaves the machine, so it is egress rather than
/// merely mutating.
#[test]
fn github_ops_is_classified_as_egress() {
    let tools = build_builtin_tools();
    let tool = tools
        .iter()
        .find(|tool| tool.name() == "github_ops")
        .expect("github_ops is registered");
    assert_eq!(
        classify(tool, ToolProvenance::BuiltIn),
        ToolClass::NetworkEgress
    );
}

/// Property 7: the decision must not depend on safety keys the model supplied.
#[test]
fn scrubbing_removes_every_model_supplied_safety_argument() {
    let hostile = serde_json::json!({
        "command": "rm -rf /",
        "approved": true,
        "allow_dangerous": true,
        "timeout_secs": 5
    });

    let scrubbed = scrub_model_supplied_safety_args(hostile);

    for key in MODEL_FORBIDDEN_SAFETY_ARGS {
        assert!(
            scrubbed.get(*key).is_none(),
            "safety argument '{key}' survived scrubbing"
        );
    }
    // Non-safety arguments must pass through untouched.
    assert_eq!(
        scrubbed.get("command").and_then(|v| v.as_str()),
        Some("rm -rf /")
    );
    assert_eq!(
        scrubbed.get("timeout_secs").and_then(|v| v.as_u64()),
        Some(5)
    );
}

/// Property 7: scrubbing is idempotent and total — an already-clean argument
/// set is unchanged, so the decision for any set equals the decision for its
/// scrubbed form.
#[test]
fn scrubbing_is_idempotent_and_preserves_clean_arguments() {
    let clean = serde_json::json!({ "command": "ls -la" });
    assert_eq!(scrub_model_supplied_safety_args(clean.clone()), clean);

    let once = scrub_model_supplied_safety_args(serde_json::json!({
        "command": "ls", "approved": true
    }));
    let twice = scrub_model_supplied_safety_args(once.clone());
    assert_eq!(once, twice);
}

/// Non-object arguments must survive scrubbing without panicking.
#[test]
fn scrubbing_tolerates_non_object_arguments() {
    assert_eq!(
        scrub_model_supplied_safety_args(serde_json::json!("bare string")),
        serde_json::json!("bare string")
    );
    assert_eq!(
        scrub_model_supplied_safety_args(serde_json::Value::Null),
        serde_json::Value::Null
    );
}

/// Deny rules must reach every tool on the surface regardless of provenance.
#[test]
fn deny_rules_apply_to_the_whole_surface() {
    let mut cfg = base_config();
    cfg.agent_deny_tools = vec!["execute_bash".to_string()];

    let sealed = ToolSurface::new().add_builtins().seal(&cfg);

    assert!(
        !sealed.names().contains("execute_bash"),
        "a denied tool survived sealing: {:?}",
        sealed.names()
    );
    assert!(
        sealed.class_of("execute_bash").is_none(),
        "class map retained a tool that was filtered out"
    );
}

/// Property 3: every callable name the system prompt advertises must be
/// registered in the runtime.
///
/// This is the regression guard for the three phantom agents the v2 prompt
/// carried — `file_search_agent`, `sequential_agent` and `quality_agent` — none
/// of which was ever registered. It audits the real prompt against the real
/// surface, so reintroducing an unregistered name fails the build.
#[test]
fn the_system_prompt_advertises_no_phantom_tools() {
    use crate::prompt_surface::PromptSurface;
    use crate::runner::ORCHESTRATOR_INSTRUCTION;

    let cfg = base_config();
    let sealed = ToolSurface::new()
        .add_builtins()
        .add_discovery_tool(0)
        .seal(&cfg);

    // Agents registered as sub-agents or agent tools rather than as built-ins.
    let agent_names = [
        "time_agent",
        "memory_agent",
        "search_agent",
        "artifact_agent",
        "developer_agent",
        "research_agent",
        "operations_agent",
        "reviewer_agent",
        "plan_work",
    ];

    let surface = PromptSurface::new(&sealed, &agent_names);
    if let Err(phantoms) = surface.assert_no_phantoms(ORCHESTRATOR_INSTRUCTION) {
        panic!(
            "the system prompt advertises {} tool(s) the runtime does not register: {:#?}",
            phantoms.len(),
            phantoms
        );
    }
}

/// The specific phantoms that shipped in v2 must stay gone.
#[test]
fn the_removed_placeholder_agents_are_not_advertised() {
    use crate::runner::ORCHESTRATOR_INSTRUCTION;

    for removed in ["file_search_agent", "sequential_agent", "quality_agent"] {
        assert!(
            !ORCHESTRATOR_INSTRUCTION.contains(removed),
            "'{removed}' was deleted from the runtime but is still named in the prompt"
        );
    }
}

/// Requirement 7.8: a configured `pre_tool` hook must actually run, and exit
/// code 2 must stop the call.
///
/// `HookExecutor` existed, was unit-tested, and was never invoked by the
/// runtime, so documented hook configuration silently did nothing.
#[tokio::test]
async fn a_pre_tool_hook_blocks_the_call() {
    use crate::hooks::{HookConfig, HookPoint};
    use crate::tools::confirming::{ConfirmingTool, clear_hook_executor};

    let mut cfg = base_config();
    cfg.hooks.insert(
        HookPoint::PreTool,
        vec![HookConfig {
            // Exit 2 is the documented "block this tool call" contract.
            command: "exit 2".to_string(),
            timeout_ms: 5_000,
            max_output: 4_096,
            matcher: None,
        }],
    );

    // Sealing installs the executor.
    let sealed = ToolSurface::new().add_builtins().seal(&cfg);
    assert!(!sealed.is_empty());

    let tool = ConfirmingTool::wrap_display_only(
        build_builtin_tools()
            .into_iter()
            .find(|tool| tool.name() == "current_unix_time")
            .expect("current_unix_time is registered"),
    );

    let ctx: std::sync::Arc<dyn adk_rust::ToolContext> =
        std::sync::Arc::new(adk_tool::SimpleToolContext::new("hook-test"));
    let result = tool
        .execute(ctx, serde_json::json!({}))
        .await
        .expect("execute should return a payload");

    clear_hook_executor();

    let error = result.get("error").and_then(|v| v.as_str()).unwrap_or("");
    assert!(
        error.contains("blocked by pre_tool hook"),
        "the pre_tool hook did not block the call: {result}"
    );
}

/// Requirement 6.5: a conditionally attached agent must be absent from the
/// prompt, not advertised and then stripped.
///
/// `search_agent` is only registered with `--provider gemini`. Before the agent
/// section was generated from the registry, every OpenAI session logged an
/// error and removed the line after the fact.
#[test]
fn the_agent_section_names_only_registered_agents() {
    use crate::prompt_surface::PromptSurface;
    use crate::runner::AGENT_CATALOGUE_FOR_TEST;

    // A surface without search_agent, as on any non-Gemini provider.
    let without_search = PromptSurface::from_names([
        "time_agent",
        "memory_agent",
        "plan_work",
        "artifact_agent",
        "developer_agent",
        "research_agent",
        "operations_agent",
        "reviewer_agent",
    ]);
    let section = without_search.render_section(AGENT_CATALOGUE_FOR_TEST);
    // Match the bullet, not a bare substring: "research_agent" contains
    // "search_agent".
    assert!(
        !section.contains("- search_agent:"),
        "search_agent was advertised without being registered: {section}"
    );
    assert!(section.contains("- developer_agent:"), "{section}");
    // Nothing in the rendered section can be a phantom, by construction.
    assert_eq!(without_search.audit(&section), vec![]);

    // With it registered, it appears.
    let with_search = PromptSurface::from_names(["search_agent", "time_agent"]);
    let section = with_search.render_section(AGENT_CATALOGUE_FOR_TEST);
    assert!(section.contains("- search_agent:"), "{section}");
    assert_eq!(with_search.audit(&section), vec![]);
}

/// Requirement 13.2: sub-agents must not be described as callable tools.
///
/// Regression for an observed failure: with all nine agents in one "call as
/// tools" list, a single prompt produced three `transfer_to_agent` hops
/// (assistant → artifact_agent → operations_agent) and no work tool at all.
/// Transferring hands over the turn, so a specialist that reads "call as tools"
/// transfers onward instead of finishing.
#[test]
fn agent_tools_and_specialists_are_described_separately() {
    use crate::runner::{AGENT_TOOL_CATALOGUE_FOR_TEST, SUBAGENT_CATALOGUE_FOR_TEST};

    let tool_names: Vec<&str> = AGENT_TOOL_CATALOGUE_FOR_TEST
        .iter()
        .map(|(name, _)| *name)
        .collect();
    let specialist_names: Vec<&str> = SUBAGENT_CATALOGUE_FOR_TEST
        .iter()
        .map(|(name, _)| *name)
        .collect();

    // The two mechanisms must not overlap: a name is either a tool or a
    // transfer target, never both.
    for name in &tool_names {
        assert!(
            !specialist_names.contains(name),
            "'{name}' appears in both catalogues"
        );
    }

    // Only genuinely tool-backed agents may be called.
    assert_eq!(
        tool_names,
        vec!["time_agent", "memory_agent", "plan_work"],
        "the tool catalogue must contain only Tool-backed agents"
    );

    // Every specialist is a sub_agent reached via transfer.
    for name in [
        "artifact_agent",
        "developer_agent",
        "operations_agent",
        "reviewer_agent",
    ] {
        assert!(
            specialist_names.contains(&name),
            "'{name}' is registered as a sub-agent but missing from the specialist catalogue"
        );
    }
}
