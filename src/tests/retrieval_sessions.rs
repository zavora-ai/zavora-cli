//! Retrieval, error redaction, session lifecycle, and route-switch tests.

use super::*;

#[test]
fn augment_prompt_with_retrieval_leaves_prompt_unchanged_when_disabled() {
    let retrieval = DisabledRetrievalService;
    let prompt = "Plan release milestones";
    let out = augment_prompt_with_retrieval(
        &retrieval,
        prompt,
        RetrievalPolicy {
            max_chunks: 3,
            max_chars: 4000,
            min_score: 1,
        },
    )
    .expect("prompt augmentation should pass");
    assert_eq!(out, prompt);
}

#[test]
fn local_file_retrieval_returns_relevant_chunks() {
    let dir = tempdir().expect("temp directory should create");
    let path = dir.path().join("knowledge.txt");
    std::fs::write(
        &path,
        "Rust CLI release planning\n\nADK retrieval abstraction and context injection",
    )
    .expect("doc file should write");

    let retrieval = LocalFileRetrievalService::load(path.to_string_lossy().as_ref())
        .expect("local retrieval should load");
    let chunks = retrieval
        .retrieve("retrieval abstraction", 3)
        .expect("retrieval should run");
    assert!(!chunks.is_empty(), "expected at least one relevant chunk");
}

#[test]
fn local_file_retrieval_ranks_chunks_deterministically_by_term_hits() {
    let retrieval = LocalFileRetrievalService {
        chunks: vec![
            RetrievedChunk {
                source: "rank:1".to_string(),
                text: "release quality gates".to_string(),
                score: 0,
            },
            RetrievedChunk {
                source: "rank:2".to_string(),
                text: "release quality gates release quality".to_string(),
                score: 0,
            },
        ],
    };

    let chunks = retrieval
        .retrieve("release quality", 2)
        .expect("retrieval should run");
    assert_eq!(chunks.len(), 2);
    assert_eq!(chunks[0].source, "rank:2");
    assert_eq!(chunks[1].source, "rank:1");
}

#[test]
fn local_retrieval_backend_requires_doc_path() {
    let mut cfg = base_cfg();
    cfg.retrieval_backend = RetrievalBackend::Local;
    cfg.retrieval_doc_path = None;

    let err = match build_retrieval_service(&cfg) {
        Ok(_) => panic!("missing doc path should fail"),
        Err(err) => err,
    };
    assert!(
        err.to_string()
            .contains("retrieval backend 'local' requires")
    );
}

#[test]
fn local_retrieval_backend_missing_file_is_reported() {
    let mut cfg = base_cfg();
    cfg.retrieval_backend = RetrievalBackend::Local;
    cfg.retrieval_doc_path = Some("does-not-exist.md".to_string());

    let err = match build_retrieval_service(&cfg) {
        Ok(_) => panic!("missing retrieval file should fail"),
        Err(err) => err,
    };
    assert!(
        err.to_string().contains("failed to read retrieval doc"),
        "expected backend unavailability error path"
    );
}

#[test]
fn retrieval_policy_enforces_context_budget_and_score_threshold() {
    let retrieval = LocalFileRetrievalService {
        chunks: vec![
            RetrievedChunk {
                source: "test:1".to_string(),
                text: "alpha beta gamma delta".to_string(),
                score: 10,
            },
            RetrievedChunk {
                source: "test:2".to_string(),
                text: "small".to_string(),
                score: 1,
            },
        ],
    };

    let out = augment_prompt_with_retrieval(
        &retrieval,
        "alpha",
        RetrievalPolicy {
            max_chunks: 3,
            max_chars: 5,
            min_score: 2,
        },
    )
    .expect("augmentation should pass");

    assert!(
        out.contains("alpha"),
        "expected retained high-score context"
    );
    assert!(
        !out.contains("small"),
        "expected low-score chunk to be filtered"
    );
}

#[test]
fn retrieval_augmentation_falls_back_when_no_matches() {
    let retrieval = LocalFileRetrievalService {
        chunks: vec![RetrievedChunk {
            source: "fallback:1".to_string(),
            text: "unrelated content".to_string(),
            score: 0,
        }],
    };

    let prompt = "release rollout";
    let out = augment_prompt_with_retrieval(
        &retrieval,
        prompt,
        RetrievalPolicy {
            max_chunks: 3,
            max_chars: 4000,
            min_score: 1,
        },
    )
    .expect("augmentation should pass");
    assert_eq!(
        out, prompt,
        "no-result path should preserve original prompt"
    );
}

#[cfg(not(feature = "semantic-search"))]
#[test]
fn semantic_retrieval_backend_requires_feature_flag() {
    let mut cfg = base_cfg();
    cfg.retrieval_backend = RetrievalBackend::Semantic;
    cfg.retrieval_doc_path = Some("README.md".to_string());
    let err = match build_retrieval_service(&cfg) {
        Ok(_) => panic!("semantic retrieval should require feature flag"),
        Err(err) => err,
    };
    assert!(
        err.to_string()
            .contains("requires feature 'semantic-search'")
    );
}

#[cfg(feature = "semantic-search")]
#[test]
fn semantic_retrieval_backend_returns_ranked_chunks() {
    let dir = tempdir().expect("temp directory should create");
    let path = dir.path().join("knowledge.txt");
    std::fs::write(
        &path,
        "Agile release planning and rollout gates\n\nSemantic retrieval context ranking",
    )
    .expect("doc file should write");

    let retrieval = SemanticLocalRetrievalService::load(path.to_string_lossy().as_ref())
        .expect("semantic retrieval should load");
    let chunks = retrieval
        .retrieve("rollout gates", 2)
        .expect("semantic retrieval should run");
    assert!(!chunks.is_empty(), "expected semantic retrieval matches");
}

#[tokio::test]
async fn sessions_show_missing_session_returns_session_category_error() {
    let cfg = base_cfg();
    let err = run_sessions_show(&cfg, Some("missing-session".to_string()), 10)
        .await
        .expect_err("missing session should error");

    assert_eq!(categorize_error(&err), ErrorCategory::Session);
    let rendered = format_cli_error(&err, cfg.show_sensitive_config);
    assert!(
        rendered.contains("[SESSION]"),
        "expected session category marker in error output"
    );
}

#[test]
fn redact_sensitive_text_masks_sqlite_urls() {
    let raw = "open failed at sqlite://.zavora/sessions.db; retry sqlite://tmp/test.db";
    let rendered = redact_sensitive_text(raw);

    assert!(!rendered.contains(".zavora/sessions.db"));
    assert!(!rendered.contains("tmp/test.db"));
    assert_eq!(
        rendered,
        "open failed at sqlite://[REDACTED]; retry sqlite://[REDACTED]"
    );
}

#[test]
fn format_cli_error_redacts_sqlite_urls_by_default() {
    let err = anyhow::anyhow!("failed to open sqlite://.zavora/sessions.db");
    let rendered = format_cli_error(&err, false);

    assert!(rendered.contains("sqlite://[REDACTED]"));
    assert!(!rendered.contains(".zavora/sessions.db"));
}

#[tokio::test]
async fn sessions_delete_requires_force_flag() {
    let (_dir, cfg) = sqlite_cfg("default-session");
    create_session(&cfg, "delete-me").await;

    let err = run_sessions_delete(&cfg, Some("delete-me".to_string()), false)
        .await
        .expect_err("delete without --force should fail");
    assert_eq!(categorize_error(&err), ErrorCategory::Input);

    let sessions = list_session_ids(&cfg).await;
    assert!(sessions.contains(&"delete-me".to_string()));
}

#[tokio::test]
async fn sessions_delete_force_removes_target_session() {
    let (_dir, cfg) = sqlite_cfg("default-session");
    create_session(&cfg, "delete-me").await;

    run_sessions_delete(&cfg, Some("delete-me".to_string()), true)
        .await
        .expect("forced delete should pass");

    let sessions = list_session_ids(&cfg).await;
    assert!(!sessions.contains(&"delete-me".to_string()));
}

#[tokio::test]
async fn sessions_prune_enforces_safety_and_deletes_when_forced() {
    let (_dir, cfg) = sqlite_cfg("default-session");
    create_session(&cfg, "s1").await;
    create_session(&cfg, "s2").await;
    create_session(&cfg, "s3").await;

    let err = run_sessions_prune(&cfg, 1, false, false)
        .await
        .expect_err("prune without --force should fail");
    assert_eq!(categorize_error(&err), ErrorCategory::Input);

    run_sessions_prune(&cfg, 1, true, false)
        .await
        .expect("dry run should pass");
    let sessions_after_dry_run = list_session_ids(&cfg).await;
    assert_eq!(sessions_after_dry_run.len(), 3);

    run_sessions_prune(&cfg, 1, false, true)
        .await
        .expect("forced prune should pass");
    let sessions_after_force = list_session_ids(&cfg).await;
    assert_eq!(sessions_after_force.len(), 1);
}

#[tokio::test]
async fn shared_memory_session_service_preserves_history_across_runner_rebuilds() {
    let cfg = base_cfg();
    let telemetry = test_telemetry(&cfg);
    let session_service: Arc<dyn SessionService> = Arc::new(InMemorySessionService::new());

    let runner_one = build_runner_with_session_service(
        build_single_agent(mock_model("first answer")).expect("agent should build"),
        &cfg,
        session_service.clone(),
        None,
    )
    .await
    .expect("runner should build");
    run_prompt(&runner_one, &cfg, "first prompt", &telemetry)
        .await
        .expect("first prompt should run");

    let runner_two = build_runner_with_session_service(
        build_single_agent(mock_model("second answer")).expect("agent should build"),
        &cfg,
        session_service.clone(),
        None,
    )
    .await
    .expect("second runner should build");
    run_prompt(&runner_two, &cfg, "second prompt", &telemetry)
        .await
        .expect("second prompt should run");

    let session = session_service
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
        "expected in-memory session history to persist across runner rebuilds"
    );
}

#[tokio::test]
async fn chat_switch_path_builds_runner_for_ollama_without_losing_session_service() {
    let mut cfg = base_cfg();
    cfg.provider = Provider::Ollama;
    cfg.model = Some("llama3.2".to_string());
    cfg.worker_provider = Provider::Ollama;
    cfg.worker_model = "llama3.2".to_string();
    let session_service: Arc<dyn SessionService> = Arc::new(InMemorySessionService::new());
    let runtime_tools = ResolvedRuntimeTools::for_test(build_builtin_tools(), BTreeSet::new());
    let tool_confirmation = ToolConfirmationSettings::default();
    let telemetry = test_telemetry(&cfg);

    let (_runner, provider, model_name) = build_single_runner_for_chat(
        &cfg,
        session_service.clone(),
        &runtime_tools,
        &tool_confirmation,
        &telemetry,
    )
    .await
    .expect("chat runner should build for ollama");

    assert_eq!(provider, Provider::Ollama);
    assert_eq!(model_name, "llama3.2");
}
