# Differentiation Roadmap — zavora-cli

Capabilities where zavora-cli exceeds or diverges from the Q CLI reference baseline.

## Current Differentiators (v1.1.0)

### 1. ADK-Native Architecture
- Built directly on ADK-Rust agent/tool/session abstractions
- Native support for all ADK workflow modes (single, sequential, parallel, loop, graph)
- ADK compaction config wired into runner — auto-compaction is framework-level, not bolted on

### 2. Multi-Provider Runtime
- 6 providers supported out of the box (gemini, openai, anthropic, deepseek, groq, ollama)
- In-session provider/model switching with validation
- Provider-aware context window defaults

### 3. Graph Workflow Engine
- Conditional routing with reusable templates
- Route classifier for dynamic agent dispatch
- Not present in Q CLI reference

### 4. Retrieval Abstraction
- Pluggable retrieval backends (disabled, local, semantic)
- Feature-gated semantic search
- Prompt enrichment integrated into all execution paths

### 5. Server Mode and A2A
- Axum-based HTTP server with structured API
- Agent-to-agent messaging protocol
- Not present in Q CLI reference chat mode

### 6. Hook Lifecycle System
- 5 hook points (agent_spawn, prompt_submit, pre_tool, post_tool, stop)
- Pre-tool blocking via exit codes
- Matcher-based scoping with wildcard patterns

## Planned Differentiators (v1.2.0+)

### 7. Delegate Sub-Agent Runner
- Full sub-agent execution with isolated sessions
- Status monitoring and result aggregation
- Multi-agent task decomposition

### 8. Semantic Retrieval Integration
- Vector-based document retrieval for context enrichment
- Hybrid keyword + semantic ranking
- Per-agent retrieval configuration

### 9. Plugin System
- ADK plugin manager integration
- Before/after run hooks at framework level
- Third-party tool registration

### 10. Advanced Compaction
- LLM-based summarization (upgrade from text extraction)
- Selective compaction (preserve tool results, compact narrative)
- Cross-session context transfer

## Competitive Position

| Area | Q CLI | zavora-cli | Advantage |
|------|-------|------------|-----------|
| Provider support | AWS-focused | 6 providers | zavora |
| Workflow modes | Single agent | 5 modes (incl. graph) | zavora |
| Server mode | No | Yes (Axum + A2A) | zavora |
| Hook system | Limited | 5-point lifecycle | zavora |
| Retrieval | No | Pluggable backends | zavora |
| Checkpoint | Shadow git repo | Conversation snapshots | Q CLI (filesystem) |
| Delegate | Background processes | Experimental | Q CLI (maturity) |
| Todo lists | File-based + tool | File-based + command | Parity |
| Compaction | LLM summary | Text extraction + ADK auto | Q CLI (quality) |
| Context tracking | Token counter | Token counter + budget UX | Parity |
