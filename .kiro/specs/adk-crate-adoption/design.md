# Design: ADK Crate Adoption for Zavora-CLI

## Overview

Each ADK crate integrates via the `Runner::builder()` chain or as registered tools. The design prioritizes minimal code — most adoptions are 1-5 lines of wiring, replacing hundreds of lines of hand-rolled code.

---

## D-1: Skill System (REQ-1)

### Integration

```rust
// In runner.rs build_runner_with_session_service()
let mut builder = Runner::builder()
    .app_name(cfg.app_name.clone())
    .agent(agent)
    .session_service(session_service)
    .with_auto_skills(".", SkillInjectorConfig::default());  // ← one line
```

### Skill File Format (`.zavora/skills/rust-expert.md`)

```markdown
---
name: rust-expert
description: Rust development best practices
triggers:
  - "*.rs"
  - Cargo.toml
---

When working with Rust code:
- Use `cargo clippy` before committing
- Prefer `thiserror` for library errors, `anyhow` for applications
- Use `#[must_use]` on functions that return Results
```

### CLI Command

```rust
// In cli.rs
Commands::Skills { command } => match command {
    SkillCommands::List => run_skills_list(),
}
```

### Dependencies

```toml
adk-skill = { path = "../adk-rust/adk-skill" }
```

### Files Changed

- `Cargo.toml` — add `adk-skill`
- `src/runner.rs` — add `.with_auto_skills()` to builder
- `src/cli.rs` — add `Skills` command
- `src/main.rs` — wire skills list

---

## D-2: Semantic Memory (REQ-2)

### Integration

```rust
// In runner.rs build_runner_with_session_service()
let memory = adk_memory::SqliteMemoryService::new(".zavora/memory.db").await?;
memory.migrate().await?;

Runner::builder()
    .memory_service(Arc::new(memory))  // ← one line
```

### Migration from memory.json

```rust
// One-time migration on first run
fn migrate_json_to_sqlite(json_path: &Path, db: &SqliteMemoryService) -> Result<()> {
    if !json_path.exists() { return Ok(()); }
    let entries: Vec<MemoryEntry> = serde_json::from_str(&std::fs::read_to_string(json_path)?)?;
    for entry in entries {
        db.add(entry.text, entry.tags, entry.score).await?;
    }
    std::fs::rename(json_path, json_path.with_extension("json.migrated"))?;
    Ok(())
}
```

### MemoryAgentTool Update

The existing `MemoryAgentTool` delegates to the ADK memory service via `ToolContext::search_memory()` which the runner now routes to `SqliteMemoryService`. The tool itself needs minimal changes — just remove the direct JSON file I/O.

### Dependencies

```toml
adk-memory = { path = "../adk-rust/adk-memory", features = ["sqlite-memory"] }
```

### Files Changed

- `Cargo.toml` — add `adk-memory`
- `src/runner.rs` — create SqliteMemoryService, pass to builder
- `src/agents/tools.rs` — simplify MemoryAgentTool to use ToolContext
- `src/agents/memory.rs` — remove JSON file I/O (or keep as fallback)

---

## D-3: OpenTelemetry (REQ-3)

### Integration

```rust
// In main.rs, before run_cli()
adk_telemetry::init_telemetry("zavora-cli")?;

// At exit
adk_telemetry::shutdown_telemetry();
```

### TelemetrySink Wrapper

Keep the existing `TelemetrySink` API but delegate to OpenTelemetry:

```rust
impl TelemetrySink {
    pub fn emit(&self, event_name: &str, metadata: Value) {
        // Emit as OpenTelemetry event on current span
        tracing::info!(
            event = event_name,
            metadata = %metadata,
            "telemetry"
        );
        // Also write to JSONL for backward compat (if enabled)
        if self.jsonl_enabled {
            self.write_jsonl(event_name, &metadata);
        }
    }
}
```

### OTLP Configuration

```bash
# Export to Jaeger/Grafana/etc
OTEL_EXPORTER_OTLP_ENDPOINT=http://localhost:4317 zavora-cli chat
```

### Dependencies

```toml
adk-telemetry = { path = "../adk-rust/adk-telemetry" }
```

### Files Changed

- `Cargo.toml` — add `adk-telemetry`
- `src/main.rs` — init/shutdown telemetry
- `src/telemetry.rs` — wrap TelemetrySink with OTel

---

## D-4: Guardrails (REQ-4)

### Integration

Replace the hand-rolled `apply_guardrail()` with ADK guardrails:

```rust
use adk_guardrail::{PiiRedactor, ContentFilter, Guardrail};

fn build_guardrails(cfg: &RuntimeConfig) -> Vec<Box<dyn Guardrail>> {
    let mut guards: Vec<Box<dyn Guardrail>> = vec![];
    if cfg.guardrail_pii_redaction {
        guards.push(Box::new(PiiRedactor::new()));
    }
    match cfg.guardrail_input_mode {
        GuardrailMode::Block => guards.push(Box::new(ContentFilter::harmful_content())),
        GuardrailMode::Redact => guards.push(Box::new(ContentFilter::blocked_keywords(
            cfg.guardrail_terms.clone(),
        ))),
        _ => {}
    }
    guards
}
```

### Dependencies

```toml
adk-guardrail = { path = "../adk-rust/adk-guardrail" }
```

### Files Changed

- `Cargo.toml` — add `adk-guardrail`
- `src/guardrail.rs` — replace regex with PiiRedactor + ContentFilter
- `src/config.rs` — add `guardrail_pii_redaction` field

---

## D-5: Plugin System + File History (REQ-5)

### Integration

```rust
use adk_plugin::{Plugin, PluginConfig, PluginManager};

fn build_file_history_plugin() -> Plugin {
    Plugin::new(PluginConfig {
        name: "file-history".to_string(),
        before_tool: Some(Box::new(|ctx, tool_name, args| {
            Box::pin(async move {
                // Snapshot file before write tools
                if matches!(tool_name, "fs_write" | "file_edit") {
                    if let Some(path) = extract_path_from_args(tool_name, args) {
                        snapshot_file(&path).await?;
                    }
                }
                Ok(())
            })
        })),
        ..Default::default()
    })
}

// In runner.rs
let mut pm = PluginManager::new();
if cfg.file_history_enabled {
    pm.register(build_file_history_plugin());
}
Runner::builder().plugin_manager(Arc::new(pm))
```

### File History Storage

```
.zavora/file_history/
├── src/main.rs/
│   ├── 1712345678.snapshot    # timestamp-named snapshots
│   ├── 1712345690.snapshot
│   └── ...                    # max 20 per file, oldest pruned
```

### /undo Command

```rust
ChatCommand::Undo => {
    let last = get_last_snapshot()?;
    std::fs::write(&last.path, &last.content)?;
    println!("Restored {} to pre-edit state", last.path);
}
```

### Dependencies

```toml
adk-plugin = { path = "../adk-rust/adk-plugin" }
```

### Files Changed

- `Cargo.toml` — add `adk-plugin`
- `src/runner.rs` — create PluginManager, register file-history plugin
- `src/chat.rs` — add `/undo` command
- New: `src/file_history.rs` — snapshot/restore logic

---

## D-6: Browser Automation (REQ-6)

### Integration

```rust
// Feature-gated
#[cfg(feature = "browser")]
fn register_browser_tools(tools: &mut Vec<Arc<dyn Tool>>, session: Arc<BrowserSession>) {
    tools.push(Arc::new(NavigateTool::new(session.clone())));
    tools.push(Arc::new(ClickTool::new(session.clone())));
    tools.push(Arc::new(TypeTextTool::new(session.clone())));
    tools.push(Arc::new(ExtractTool::new(session.clone())));
    tools.push(Arc::new(ScreenshotTool::new(session.clone())));
    tools.push(Arc::new(WaitTool::new(session.clone())));
    tools.push(Arc::new(EvaluateTool::new(session.clone())));
    tools.push(Arc::new(CookiesTool::new(session.clone())));
}
```

### Lazy Session

```rust
static BROWSER_SESSION: OnceCell<Arc<BrowserSession>> = OnceCell::const_new();

async fn get_browser_session() -> Result<Arc<BrowserSession>> {
    BROWSER_SESSION.get_or_try_init(|| async {
        let session = BrowserSession::new(BrowserConfig::default()).await?;
        Ok(Arc::new(session))
    }).await.cloned()
}
```

### Dependencies

```toml
adk-browser = { path = "../adk-rust/adk-browser", optional = true }

[features]
browser = ["dep:adk-browser"]
```

### Files Changed

- `Cargo.toml` — add `adk-browser` (optional)
- `src/tools/mod.rs` — register browser tools (feature-gated)
- `src/runner.rs` — system prompt update for browser tools

---

## D-7: Code Sandbox (REQ-7)

### Integration

```rust
#[cfg(feature = "sandbox")]
fn build_sandbox_tool() -> Arc<dyn Tool> {
    let backend = Arc::new(ProcessBackend::new(ProcessConfig {
        timeout: Duration::from_secs(30),
        ..Default::default()
    }));
    Arc::new(SandboxTool::new(backend))
}
```

### Dependencies

```toml
adk-code = { path = "../adk-rust/adk-code", optional = true }
adk-sandbox = { path = "../adk-rust/adk-sandbox", optional = true, features = ["process"] }

[features]
sandbox = ["dep:adk-code", "dep:adk-sandbox"]
```

### Files Changed

- `Cargo.toml` — add `adk-code`, `adk-sandbox` (optional)
- `src/tools/mod.rs` — register sandbox tool (feature-gated)

---

## D-8: RAG Pipeline (REQ-8)

### Integration

```rust
use adk_rag::{RagPipeline, InMemoryVectorStore, Document};

async fn build_rag_pipeline(cfg: &RuntimeConfig) -> Result<RagPipeline> {
    let store = Arc::new(InMemoryVectorStore::new());
    let pipeline = RagPipeline::builder()
        .vector_store(store)
        .build()?;

    // Ingest configured doc paths
    if let Some(doc_path) = &cfg.retrieval_doc_path {
        let doc = Document::from_file(doc_path).await?;
        pipeline.ingest("default", &doc).await?;
    }

    Ok(pipeline)
}

// Replace retrieval augmentation
async fn augment_with_rag(pipeline: &RagPipeline, prompt: &str) -> Result<String> {
    let results = pipeline.query("default", prompt).await?;
    if results.is_empty() { return Ok(prompt.to_string()); }
    let context = results.iter().map(|r| r.text.as_str()).collect::<Vec<_>>().join("\n\n");
    Ok(format!("<context>\n{}\n</context>\n\n{}", context, prompt))
}
```

### CLI Command

```bash
zavora-cli rag ingest ./docs/
zavora-cli rag ingest ./README.md
```

### Dependencies

```toml
adk-rag = { path = "../adk-rust/adk-rag", optional = true, features = ["inmemory"] }

[features]
rag = ["dep:adk-rag"]
```

### Files Changed

- `Cargo.toml` — add `adk-rag` (optional)
- `src/retrieval.rs` — replace with RagPipeline
- `src/cli.rs` — add `Rag` command
- `src/main.rs` — wire rag ingest

---

## Dependency Summary

### Phase 1 (always included)
```toml
adk-skill = { path = "../adk-rust/adk-skill" }
adk-memory = { path = "../adk-rust/adk-memory", features = ["sqlite-memory"] }
adk-telemetry = { path = "../adk-rust/adk-telemetry" }
```

### Phase 2 (always included)
```toml
adk-guardrail = { path = "../adk-rust/adk-guardrail" }
adk-plugin = { path = "../adk-rust/adk-plugin" }
```

### Phase 3 (feature-gated)
```toml
adk-browser = { path = "../adk-rust/adk-browser", optional = true }
adk-code = { path = "../adk-rust/adk-code", optional = true }
adk-sandbox = { path = "../adk-rust/adk-sandbox", optional = true, features = ["process"] }
adk-rag = { path = "../adk-rust/adk-rag", optional = true, features = ["inmemory"] }

[features]
browser = ["dep:adk-browser"]
sandbox = ["dep:adk-code", "dep:adk-sandbox"]
rag = ["dep:adk-rag"]
```

---

## Code Removal

Adopting ADK crates allows removing hand-rolled code:

| File | Lines Removed | Replaced By |
|------|--------------|-------------|
| `src/agents/memory.rs` | ~150 | `adk-memory::SqliteMemoryService` |
| `src/telemetry.rs` (JSONL writer) | ~100 | `adk-telemetry` |
| `src/guardrail.rs` (regex) | ~100 | `adk-guardrail` |
| `src/retrieval.rs` (local search) | ~150 | `adk-rag::RagPipeline` |
| **Total** | **~500 lines removed** | **5 crate dependencies** |
