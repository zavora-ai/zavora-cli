use std::collections::HashMap;
use std::net::SocketAddr;
use std::sync::Arc;
use std::time::{Duration, Instant};

use adk_rust::prelude::*;
use adk_session::SessionService;
use anyhow::{Context, Result};
use axum::extract::State;
use axum::http::StatusCode;
use axum::routing::{get, post};
use axum::{Json, Router as AxumRouter};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::config::RuntimeConfig;
use crate::eval::round_metric;
use crate::guardrail::{apply_guardrail, enforce_prompt_limit};
use crate::provider::resolve_model;
use crate::retrieval::{RetrievalService, build_retrieval_service};
use crate::runner::{
    build_runner_with_session_service, build_single_agent_with_tools, resolve_runtime_tools,
    resolve_tool_confirmation_settings,
};
use crate::session::build_session_service;
use crate::streaming::run_prompt_with_retrieval;
use crate::telemetry::TelemetrySink;
#[derive(Clone)]
pub struct ServerState {
    pub cfg: RuntimeConfig,
    pub retrieval: Arc<dyn RetrievalService>,
    pub telemetry: TelemetrySink,
    pub server_agent: Arc<dyn Agent>,
    pub session_service: Arc<dyn SessionService>,
    pub run_config: RunConfig,
    pub provider_label: String,
    pub model_name: String,
    pub runner_cache: Arc<tokio::sync::RwLock<HashMap<String, Arc<Runner>>>>,
    pub auth_token: Option<String>,
    pub runner_cache_max: usize,
}

#[derive(Debug, Serialize)]
pub struct ServerHealthResponse {
    pub status: &'static str,
    pub app_name: String,
    pub profile: String,
}

#[derive(Debug, Deserialize)]
pub struct ServerAskRequest {
    pub prompt: String,
    pub session_id: Option<String>,
    pub user_id: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct ServerAskResponse {
    pub answer: String,
    pub provider: String,
    pub model: String,
    pub session_id: String,
    pub user_id: String,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct A2aPingRequest {
    pub from_agent: String,
    pub to_agent: String,
    pub message_id: String,
    pub correlation_id: Option<String>,
    #[serde(default)]
    pub payload: Value,
}

#[derive(Debug, Serialize)]
pub struct A2aPingResponse {
    pub from_agent: String,
    pub to_agent: String,
    pub message_id: String,
    pub correlation_id: String,
    pub acknowledged_message_id: String,
    pub status: String,
    pub payload: Value,
}

pub type ApiError = (StatusCode, Json<Value>);
pub type ApiResult<T> = std::result::Result<Json<T>, ApiError>;

pub fn api_error(status: StatusCode, message: impl Into<String>) -> ApiError {
    (status, Json(json!({ "error": message.into() })))
}

pub fn server_runner_cache_key(cfg: &RuntimeConfig) -> String {
    format!("{}::{}", cfg.user_id, cfg.session_id)
}

pub async fn get_or_build_server_runner(
    state: &ServerState,
    cfg: &RuntimeConfig,
) -> Result<(Arc<Runner>, &'static str)> {
    let key = server_runner_cache_key(cfg);
    if let Some(runner) = state.runner_cache.read().await.get(&key).cloned() {
        return Ok((runner, "hit"));
    }

    let runner = Arc::new(
        build_runner_with_session_service(
            state.server_agent.clone(),
            cfg,
            state.session_service.clone(),
            Some(state.run_config.clone()),
        )
        .await?,
    );

    let mut cache = state.runner_cache.write().await;
    if let Some(existing) = cache.get(&key).cloned() {
        return Ok((existing, "hit-race"));
    }

    // Evict oldest entry when cache is at capacity.
    if cache.len() >= state.runner_cache_max && !cache.is_empty() {
        if let Some(evict_key) = cache.keys().next().cloned() {
            cache.remove(&evict_key);
            tracing::info!(evicted_key = %evict_key, cache_size = cache.len(), "server runner cache eviction");
        }
    }

    cache.insert(key, runner.clone());
    Ok((runner, "miss"))
}

pub fn check_server_auth(
    state: &ServerState,
    headers: &axum::http::HeaderMap,
) -> Result<(), ApiError> {
    let Some(expected_token) = state.auth_token.as_deref() else {
        return Ok(()); // no token configured, auth disabled
    };

    let header_value = headers
        .get(axum::http::header::AUTHORIZATION)
        .and_then(|v| v.to_str().ok())
        .unwrap_or_default();

    let provided_token = header_value
        .strip_prefix("Bearer ")
        .unwrap_or_default()
        .trim();

    if provided_token.is_empty() || provided_token != expected_token {
        return Err(api_error(
            StatusCode::UNAUTHORIZED,
            "missing or invalid Authorization bearer token",
        ));
    }

    Ok(())
}

pub async fn handle_server_health(
    State(state): State<Arc<ServerState>>,
) -> Json<ServerHealthResponse> {
    Json(ServerHealthResponse {
        status: "ok",
        app_name: state.cfg.app_name.clone(),
        profile: state.cfg.profile.clone(),
    })
}

pub async fn handle_server_ask(
    State(state): State<Arc<ServerState>>,
    headers: axum::http::HeaderMap,
    Json(request): Json<ServerAskRequest>,
) -> ApiResult<ServerAskResponse> {
    check_server_auth(&state, &headers)?;
    let started_at = Instant::now();
    let mut cfg = state.cfg.clone();
    if let Some(session_id) = request.session_id {
        cfg.session_id = session_id;
    }
    if let Some(user_id) = request.user_id {
        cfg.user_id = user_id;
    }

    let prompt = request.prompt.trim().to_string();
    if prompt.is_empty() {
        return Err(api_error(
            StatusCode::BAD_REQUEST,
            "prompt cannot be empty for /v1/ask",
        ));
    }

    enforce_prompt_limit(&prompt, cfg.max_prompt_chars)
        .map_err(|err| api_error(StatusCode::BAD_REQUEST, err.to_string()))?;

    let guarded_prompt = apply_guardrail(
        &cfg,
        &state.telemetry,
        "input",
        cfg.guardrail_input_mode,
        &prompt,
    )
    .map_err(|err| api_error(StatusCode::BAD_REQUEST, err.to_string()))?;

    let (runner, cache_status) = get_or_build_server_runner(&state, &cfg)
        .await
        .map_err(|err| api_error(StatusCode::INTERNAL_SERVER_ERROR, err.to_string()))?;

    let answer = run_prompt_with_retrieval(
        runner.as_ref(),
        &cfg,
        &guarded_prompt,
        state.retrieval.as_ref(),
        &state.telemetry,
    )
    .await
    .map_err(|err| api_error(StatusCode::INTERNAL_SERVER_ERROR, err.to_string()))?;
    let answer = apply_guardrail(
        &cfg,
        &state.telemetry,
        "output",
        cfg.guardrail_output_mode,
        &answer,
    )
    .map_err(|err| api_error(StatusCode::FORBIDDEN, err.to_string()))?;

    state.telemetry.emit(
        "server.ask.completed",
        json!({
            "provider": state.provider_label.clone(),
            "model": state.model_name.clone(),
            "session_id": cfg.session_id.clone(),
            "user_id": cfg.user_id.clone(),
            "runner_cache": cache_status,
            "latency_ms": round_metric(started_at.elapsed().as_secs_f64() * 1000.0)
        }),
    );

    Ok(Json(ServerAskResponse {
        answer,
        provider: state.provider_label.clone(),
        model: state.model_name.clone(),
        session_id: cfg.session_id,
        user_id: cfg.user_id,
    }))
}

pub fn process_a2a_ping(request: A2aPingRequest) -> Result<A2aPingResponse> {
    if request.from_agent.trim().is_empty() {
        return Err(anyhow::anyhow!("from_agent is required for A2A ping"));
    }
    if request.to_agent.trim().is_empty() {
        return Err(anyhow::anyhow!("to_agent is required for A2A ping"));
    }
    if request.message_id.trim().is_empty() {
        return Err(anyhow::anyhow!("message_id is required for A2A ping"));
    }

    let correlation_id = request
        .correlation_id
        .clone()
        .unwrap_or_else(|| request.message_id.clone());

    Ok(A2aPingResponse {
        from_agent: request.to_agent.clone(),
        to_agent: request.from_agent.clone(),
        message_id: format!("ack-{}", request.message_id),
        correlation_id,
        acknowledged_message_id: request.message_id,
        status: "acknowledged".to_string(),
        payload: json!({
            "accepted": true,
            "protocol": "zavora-a2a-v1"
        }),
    })
}

pub async fn handle_a2a_ping(
    State(state): State<Arc<ServerState>>,
    headers: axum::http::HeaderMap,
    Json(request): Json<A2aPingRequest>,
) -> ApiResult<A2aPingResponse> {
    check_server_auth(&state, &headers)?;
    state.telemetry.emit(
        "a2a.ping.received",
        json!({
            "from_agent": request.from_agent.clone(),
            "to_agent": request.to_agent.clone(),
            "message_id": request.message_id.clone()
        }),
    );
    let response = process_a2a_ping(request)
        .map_err(|err| api_error(StatusCode::BAD_REQUEST, err.to_string()))?;
    state.telemetry.emit(
        "a2a.ping.responded",
        json!({
            "message_id": response.message_id,
            "status": response.status
        }),
    );
    Ok(Json(response))
}

pub fn build_server_router(state: Arc<ServerState>) -> AxumRouter {
    AxumRouter::new()
        .route("/healthz", get(handle_server_health))
        .route("/v1/ask", post(handle_server_ask))
        .route("/v1/a2a/ping", post(handle_a2a_ping))
        .with_state(state)
}

pub async fn run_server(
    cfg: RuntimeConfig,
    host: String,
    port: u16,
    telemetry: &TelemetrySink,
) -> Result<()> {
    let addr = format!("{host}:{port}")
        .parse::<SocketAddr>()
        .with_context(|| format!("invalid server bind address '{}:{}'", host, port))?;
    let retrieval = build_retrieval_service(&cfg)?;
    let runtime_tools = resolve_runtime_tools(&cfg).await;
    let tool_confirmation = resolve_tool_confirmation_settings(&cfg, &runtime_tools);
    let (model, resolved_provider, model_name) = resolve_model(&cfg)?;
    let provider_label = format!("{:?}", resolved_provider).to_ascii_lowercase();
    telemetry.emit(
        "model.resolved",
        json!({
            "provider": provider_label.clone(),
            "model": model_name.clone(),
            "path": "server"
        }),
    );
    let server_agent = build_single_agent_with_tools(
        model,
        &runtime_tools.tools,
        tool_confirmation.policy.clone(),
        Duration::from_secs(cfg.tool_timeout_secs),
        Some(&cfg),
    )?;
    let session_service = build_session_service(&cfg).await?;
    let warm_runner = Arc::new(
        build_runner_with_session_service(
            server_agent.clone(),
            &cfg,
            session_service.clone(),
            Some(tool_confirmation.run_config.clone()),
        )
        .await?,
    );
    let mut runner_cache = HashMap::new();
    runner_cache.insert(server_runner_cache_key(&cfg), warm_runner);
    let state = Arc::new(ServerState {
        cfg: cfg.clone(),
        retrieval,
        telemetry: telemetry.clone(),
        server_agent,
        session_service,
        run_config: tool_confirmation.run_config.clone(),
        provider_label: provider_label.clone(),
        model_name: model_name.clone(),
        runner_cache: Arc::new(tokio::sync::RwLock::new(runner_cache)),
        auth_token: std::env::var("ZAVORA_SERVER_AUTH_TOKEN")
            .ok()
            .map(|v| v.trim().to_string())
            .filter(|v| !v.is_empty()),
        runner_cache_max: cfg.server_runner_cache_max.max(1),
    });

    telemetry.emit(
        "server.started",
        json!({
            "host": host,
            "port": port,
            "profile": cfg.profile,
            "session_backend": format!("{:?}", cfg.session_backend),
            "provider": provider_label,
            "model": model_name
        }),
    );

    println!(
        "Server mode listening on http://{} (health: /healthz, ask: /v1/ask, a2a: /v1/a2a/ping)",
        addr
    );

    let listener = tokio::net::TcpListener::bind(addr)
        .await
        .context("failed to bind server listener")?;
    axum::serve(listener, build_server_router(state))
        .with_graceful_shutdown(shutdown_signal())
        .await
        .context("server runtime failed")
}

pub async fn shutdown_signal() {
    let ctrl_c = async {
        tokio::signal::ctrl_c()
            .await
            .expect("failed to install Ctrl+C handler");
    };

    #[cfg(unix)]
    let terminate = async {
        tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())
            .expect("failed to install SIGTERM handler")
            .recv()
            .await;
    };

    #[cfg(not(unix))]
    let terminate = std::future::pending::<()>();

    tokio::select! {
        _ = ctrl_c => { println!("\nReceived Ctrl+C, shutting down gracefully..."); }
        _ = terminate => { println!("\nReceived SIGTERM, shutting down gracefully..."); }
    }
}

pub fn run_a2a_smoke(telemetry: &TelemetrySink) -> Result<()> {
    let request = A2aPingRequest {
        from_agent: "sales-agent".to_string(),
        to_agent: "procurement-agent".to_string(),
        message_id: "msg-001".to_string(),
        correlation_id: Some("corr-001".to_string()),
        payload: json!({ "intent": "supply-check" }),
    };
    let response = process_a2a_ping(request.clone())?;

    if response.acknowledged_message_id != request.message_id {
        return Err(anyhow::anyhow!(
            "a2a smoke failed: ack id '{}' does not match request id '{}'",
            response.acknowledged_message_id,
            request.message_id
        ));
    }
    if response.correlation_id != "corr-001" {
        return Err(anyhow::anyhow!(
            "a2a smoke failed: expected correlation_id corr-001 but got '{}'",
            response.correlation_id
        ));
    }

    telemetry.emit(
        "a2a.smoke.passed",
        json!({
            "from_agent": request.from_agent,
            "to_agent": request.to_agent,
            "message_id": request.message_id
        }),
    );
    println!("A2A smoke passed: basic request/ack contract is valid.");
    Ok(())
}
