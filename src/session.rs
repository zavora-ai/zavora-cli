use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use adk_rust::prelude::*;
use adk_session::*;
use anyhow::{Context, Result};

use crate::cli::SessionBackend;
use crate::config::RuntimeConfig;
use crate::streaming::event_text;

pub async fn build_session_service(cfg: &RuntimeConfig) -> Result<Arc<dyn SessionService>> {
    match cfg.session_backend {
        SessionBackend::Memory => Ok(Arc::new(InMemorySessionService::new())),
        SessionBackend::Sqlite => {
            let service = open_sqlite_session_service(&cfg.session_db_url).await?;
            Ok(Arc::new(service))
        }
    }
}

pub async fn open_sqlite_session_service(db_url: &str) -> Result<DatabaseSessionService> {
    ensure_parent_dir_for_sqlite_url(db_url)?;
    let service = DatabaseSessionService::new(db_url)
        .await
        .context("failed to open sqlite session database")?;
    service
        .migrate()
        .await
        .context("failed to run sqlite session migrations")?;
    Ok(service)
}

pub async fn ensure_session_exists(
    session_service: &Arc<dyn SessionService>,
    cfg: &RuntimeConfig,
) -> Result<()> {
    let session = session_service
        .get(GetRequest {
            app_name: cfg.app_name.clone(),
            user_id: cfg.user_id.clone(),
            session_id: cfg.session_id.clone(),
            num_recent_events: None,
            after: None,
        })
        .await;

    if session.is_ok() {
        return Ok(());
    }

    session_service
        .create(CreateRequest {
            app_name: cfg.app_name.clone(),
            user_id: cfg.user_id.clone(),
            session_id: Some(cfg.session_id.clone()),
            state: HashMap::new(),
        })
        .await
        .with_context(|| {
            format!(
                "failed to create session '{}' for app '{}'",
                cfg.session_id, cfg.app_name
            )
        })?;

    Ok(())
}

pub fn ensure_parent_dir_for_sqlite_url(db_url: &str) -> Result<()> {
    let Some(db_path) = sqlite_path_from_url(db_url) else {
        return Ok(());
    };

    if let Some(parent) = db_path.parent()
        && !parent.as_os_str().is_empty()
    {
        std::fs::create_dir_all(parent).with_context(|| {
            format!(
                "failed to create directory for sqlite database: {}",
                parent.display()
            )
        })?;
    }

    if !db_path.exists() {
        std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(&db_path)
            .with_context(|| {
                format!(
                    "failed to initialize sqlite database file: {}",
                    db_path.display()
                )
            })?;
    }

    Ok(())
}

pub fn sqlite_path_from_url(db_url: &str) -> Option<PathBuf> {
    if !db_url.starts_with("sqlite://") {
        return None;
    }

    let path_with_params = db_url.trim_start_matches("sqlite://");
    let path_without_params = path_with_params
        .split_once('?')
        .map(|(path, _)| path)
        .unwrap_or(path_with_params);

    if path_without_params.is_empty() || path_without_params == ":memory:" {
        return None;
    }

    Some(Path::new(path_without_params).to_path_buf())
}

pub async fn run_sessions_list(cfg: &RuntimeConfig) -> Result<()> {
    let session_service = build_session_service(cfg).await?;
    let mut sessions = session_service
        .list(ListRequest {
            app_name: cfg.app_name.clone(),
            user_id: cfg.user_id.clone(),
        })
        .await
        .with_context(|| {
            format!(
                "failed to list sessions for app '{}' and user '{}'",
                cfg.app_name, cfg.user_id
            )
        })?;

    if sessions.is_empty() {
        println!(
            "No sessions found for app '{}' and user '{}'.",
            cfg.app_name, cfg.user_id
        );
        return Ok(());
    }

    sessions.sort_by_key(|session| std::cmp::Reverse(session.last_update_time()));

    println!(
        "Sessions for app '{}' and user '{}':",
        cfg.app_name, cfg.user_id
    );
    for session in sessions {
        println!(
            "- {} (updated: {})",
            session.id(),
            session.last_update_time().to_rfc3339()
        );
    }

    Ok(())
}

pub async fn run_sessions_show(
    cfg: &RuntimeConfig,
    session_id_override: Option<String>,
    recent: usize,
) -> Result<()> {
    let session_id = session_id_override.unwrap_or_else(|| cfg.session_id.clone());
    let session_service = build_session_service(cfg).await?;
    let session = session_service
        .get(GetRequest {
            app_name: cfg.app_name.clone(),
            user_id: cfg.user_id.clone(),
            session_id: session_id.clone(),
            num_recent_events: (recent > 0).then_some(recent),
            after: None,
        })
        .await
        .with_context(|| {
            format!(
                "failed to load session '{}' for app '{}' and user '{}'",
                session_id, cfg.app_name, cfg.user_id
            )
        })?;

    println!(
        "Session '{}' (app='{}', user='{}', events={}):",
        session.id(),
        session.app_name(),
        session.user_id(),
        session.events().len()
    );

    let events = session.events().all();
    if events.is_empty() {
        println!("No events in this session.");
        return Ok(());
    }

    for event in events {
        print_session_event(&event);
    }

    Ok(())
}

pub async fn run_sessions_delete(
    cfg: &RuntimeConfig,
    session_id_override: Option<String>,
    force: bool,
) -> Result<()> {
    let session_id = session_id_override.unwrap_or_else(|| cfg.session_id.clone());
    if !force {
        return Err(anyhow::anyhow!(
            "session delete is destructive. Re-run with --force to delete session '{}'",
            session_id
        ));
    }

    let session_service = build_session_service(cfg).await?;
    session_service
        .delete(DeleteRequest {
            app_name: cfg.app_name.clone(),
            user_id: cfg.user_id.clone(),
            session_id: session_id.clone(),
        })
        .await
        .with_context(|| {
            format!(
                "failed to delete session '{}' for app '{}' and user '{}'",
                session_id, cfg.app_name, cfg.user_id
            )
        })?;

    println!(
        "Deleted session '{}' for app '{}' and user '{}'.",
        session_id, cfg.app_name, cfg.user_id
    );
    Ok(())
}

pub async fn run_sessions_prune(
    cfg: &RuntimeConfig,
    keep: usize,
    dry_run: bool,
    force: bool,
) -> Result<()> {
    let keep = keep.max(1);
    let session_service = build_session_service(cfg).await?;
    let mut sessions = session_service
        .list(ListRequest {
            app_name: cfg.app_name.clone(),
            user_id: cfg.user_id.clone(),
        })
        .await
        .with_context(|| {
            format!(
                "failed to list sessions for prune in app '{}' and user '{}'",
                cfg.app_name, cfg.user_id
            )
        })?;

    sessions.sort_by_key(|session| std::cmp::Reverse(session.last_update_time()));
    let prune_ids = sessions
        .into_iter()
        .skip(keep)
        .map(|session| session.id().to_string())
        .collect::<Vec<String>>();

    if prune_ids.is_empty() {
        println!(
            "Nothing to prune. Keep={} and current session count is within limit.",
            keep
        );
        return Ok(());
    }

    if dry_run {
        println!(
            "Dry-run: {} session(s) would be deleted (keeping {} most recent):",
            prune_ids.len(),
            keep
        );
        for id in prune_ids {
            println!("- {id}");
        }
        return Ok(());
    }

    if !force {
        return Err(anyhow::anyhow!(
            "session prune is destructive and would delete {} session(s). Re-run with --force or preview with --dry-run",
            prune_ids.len()
        ));
    }

    for session_id in &prune_ids {
        session_service
            .delete(DeleteRequest {
                app_name: cfg.app_name.clone(),
                user_id: cfg.user_id.clone(),
                session_id: session_id.clone(),
            })
            .await
            .with_context(|| {
                format!(
                    "failed to delete pruned session '{}' for app '{}' and user '{}'",
                    session_id, cfg.app_name, cfg.user_id
                )
            })?;
    }

    println!(
        "Pruned {} session(s). Kept {} most recent session(s).",
        prune_ids.len(),
        keep
    );
    Ok(())
}

fn print_session_event(event: &Event) {
    let mut header = format!("[{}] {}", event.timestamp.to_rfc3339(), event.author);
    if event.is_final_response() {
        header.push_str(" [final]");
    }
    println!("{header}");

    let text = event_text(event);
    if !text.is_empty() {
        println!("{text}");
    } else {
        println!("<non-text event>");
    }

    if !event.actions.state_delta.is_empty() {
        let mut keys = event
            .actions
            .state_delta
            .keys()
            .cloned()
            .collect::<Vec<String>>();
        keys.sort();
        println!("state_delta keys: {}", keys.join(", "));
    }

    println!();
}
