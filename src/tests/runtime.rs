//! Workflow, streaming, runner, and session integration tests.

use super::*;

#[tokio::test]
async fn single_workflow_returns_deterministic_mock_output() {
    let cfg = base_cfg();
    let telemetry = test_telemetry(&cfg);
    let runner = build_runner(
        build_single_agent(mock_model("single response")).expect("agent should build"),
        &cfg,
    )
    .await
    .expect("runner should build");

    let out = run_prompt(&runner, &cfg, "hello", &telemetry)
        .await
        .expect("prompt should run");
    assert_eq!(out, "single response");
}

#[tokio::test]
async fn workflow_modes_return_deterministic_mock_output() {
    let modes = [
        WorkflowMode::Single,
        WorkflowMode::Sequential,
        WorkflowMode::Parallel,
        WorkflowMode::Loop,
        WorkflowMode::Graph,
    ];

    for mode in modes {
        let mut cfg = base_cfg();
        cfg.session_id = format!("session-{mode:?}");
        let telemetry = test_telemetry(&cfg);
        let runner = build_runner(
            build_workflow_agent(
                mode,
                mock_model("workflow response"),
                1,
                &build_builtin_tools(),
                ToolConfirmationPolicy::Never,
                Duration::from_secs(45),
                None,
            )
            .expect("workflow should build"),
            &cfg,
        )
        .await
        .expect("runner should build");

        let out = run_prompt(&runner, &cfg, "build a plan", &telemetry)
            .await
            .expect("prompt should run");
        assert_eq!(out, "workflow response");
    }
}

/// Search is available only when the invocation explicitly runs Gemini.
///
/// Tests the provider gate directly. The specialists moved from `sub_agent` to
/// `AgentTool` because ADK emits no `Part::FunctionResponse` on a successful
/// `transfer_to_agent` (adk-agent/src/llm_agent.rs), leaving an unpaired
/// function call that the OpenAI Responses API rejects with "No tool output
/// found for function call". Since the `Agent` trait exposes no tool list, the
/// gate is asserted at its source rather than through the assembled agent.
#[test]
fn search_subagent_is_only_attached_for_explicit_gemini_provider() {
    use crate::runner::build_search_subagent_for_provider;

    let mut cfg = base_cfg();

    cfg.provider = Provider::Gemini;
    assert!(
        build_search_subagent_for_provider(Some(&cfg), mock_model("gemini")).is_some(),
        "search agent should be available for explicit Gemini provider"
    );

    for provider in [Provider::Auto, Provider::Openai, Provider::Anthropic] {
        cfg.provider = provider;
        assert!(
            build_search_subagent_for_provider(Some(&cfg), mock_model("other")).is_none(),
            "search agent must not attach for {provider:?}"
        );
    }

    // No runtime config at all: nothing to gate on, so nothing attaches.
    assert!(build_search_subagent_for_provider(None, mock_model("none")).is_none());
}

/// Specialists must never be registered as transfer targets, because a
/// successful ADK transfer leaves an unpaired function call.
#[test]
fn specialists_are_not_registered_as_transfer_targets() {
    let cfg = base_cfg();
    let agent = build_single_agent_with_tools(
        mock_model("openai"),
        &[],
        ToolConfirmationPolicy::Never,
        Duration::from_secs(45),
        Some(&cfg),
    )
    .expect("agent should build");

    assert!(
        agent.sub_agents().is_empty(),
        "found {} sub-agent(s); specialists must be AgentTools so their results \
         pair with their calls: {:?}",
        agent.sub_agents().len(),
        agent
            .sub_agents()
            .iter()
            .map(|a| a.name())
            .collect::<Vec<_>>()
    );
}

#[tokio::test]
async fn sqlite_session_backend_persists_history_between_runners() {
    let dir = tempdir().expect("temp directory should create");
    let db_path = dir.path().join("sessions.db");
    let db_url = format!("sqlite://{}", db_path.to_string_lossy());

    let mut cfg = base_cfg();
    cfg.session_backend = SessionBackend::Sqlite;
    cfg.session_db_url = db_url.clone();
    cfg.session_id = "persisted-session".to_string();
    let telemetry = test_telemetry(&cfg);

    let runner_one = build_runner(
        build_single_agent(mock_model("first answer")).expect("agent should build"),
        &cfg,
    )
    .await
    .expect("runner should build");

    let _ = run_prompt(&runner_one, &cfg, "first prompt", &telemetry)
        .await
        .expect("first prompt should run");

    let runner_two = build_runner(
        build_single_agent(mock_model("second answer")).expect("agent should build"),
        &cfg,
    )
    .await
    .expect("second runner should build");

    let _ = run_prompt(&runner_two, &cfg, "second prompt", &telemetry)
        .await
        .expect("second prompt should run");

    let service = SqliteSessionService::new(&db_url)
        .await
        .expect("db should open");
    service.migrate().await.expect("migration should run");

    let session = service
        .get(GetRequest {
            app_name: cfg.app_name.clone(),
            user_id: cfg.user_id.clone(),
            session_id: cfg.session_id.clone(),
            num_recent_events: None,
            after: None,
        })
        .await
        .expect("session should exist");

    assert!(
        session.events().len() >= 4,
        "expected persisted event history across runs"
    );
}

#[test]
fn ingest_author_text_handles_partial_then_final_snapshot() {
    let mut buffer = String::new();

    let d1 = ingest_author_text(&mut buffer, "Hello", true, false);
    let d2 = ingest_author_text(&mut buffer, " world", true, false);
    let d3 = ingest_author_text(&mut buffer, "Hello world", false, true);

    assert_eq!(d1, "Hello");
    assert_eq!(d2, " world");
    assert!(d3.is_empty(), "final snapshot should not duplicate output");
    assert_eq!(buffer, "Hello world");
}

#[test]
fn ingest_author_text_handles_non_partial_incremental_chunks() {
    let mut buffer = String::new();

    let d1 = ingest_author_text(&mut buffer, "Hello", false, false);
    let d2 = ingest_author_text(&mut buffer, " world", false, false);
    let d3 = ingest_author_text(&mut buffer, "Hello world", false, true);

    assert_eq!(d1, "Hello");
    assert_eq!(d2, " world");
    assert!(d3.is_empty(), "final snapshot should be deduplicated");
    assert_eq!(buffer, "Hello world");
}

#[test]
fn tracker_falls_back_to_last_textful_author() {
    let mut tracker = AuthorTextTracker::default();

    let _ = tracker.ingest_parts("assistant", "hello", false, false);
    let _ = tracker.ingest_parts("tool", "", false, true);

    assert_eq!(tracker.resolve_text().as_deref(), Some("hello"));
}

#[test]
fn final_stream_suffix_emits_only_missing_tail() {
    assert_eq!(
        final_stream_suffix("Hello", "Hello world").as_deref(),
        Some(" world")
    );
    assert_eq!(final_stream_suffix("Hello world", "Hello world"), None);
    assert_eq!(
        final_stream_suffix("", "Hello world").as_deref(),
        Some("Hello world")
    );
}

#[test]
fn workflow_route_classifier_is_deterministic_for_key_intents() {
    assert_eq!(
        classify_workflow_route("Plan release milestones"),
        "release"
    );
    assert_eq!(
        classify_workflow_route("Evaluate architecture tradeoffs"),
        "architecture"
    );
    assert_eq!(
        classify_workflow_route("List risk mitigations and rollback"),
        "risk"
    );
    assert_eq!(
        classify_workflow_route("Implement feature work"),
        "delivery"
    );
}

#[test]
fn workflow_templates_exist_for_all_graph_routes() {
    for route in ["release", "architecture", "risk", "delivery"] {
        let template = workflow_template(route);
        assert!(
            !template.trim().is_empty(),
            "template should be non-empty for route {route}"
        );
    }
}

#[test]
fn tool_failure_extractor_handles_common_error_shapes() {
    assert_eq!(
        extract_tool_failure_message(&json!({"error": "denied by policy"})).as_deref(),
        Some("denied by policy")
    );
    assert_eq!(
        extract_tool_failure_message(&json!({"status": "error", "message": "timeout"})).as_deref(),
        Some("timeout")
    );
    assert_eq!(extract_tool_failure_message(&json!({"ok": true})), None);
}
