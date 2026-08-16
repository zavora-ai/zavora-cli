# Multi-Agent Architecture Redesign

## Overview

Redesign from weak "filtered tool" agents to **capability + workflow** agents with clear value propositions.

## Agent Tiers

### Tier 1: Capability Agents (Unique Skills)

#### 1. Search Agent 🔍
**Capability:** Web search via Gemini's built-in Google Search
**Gating:** Only available if using Gemini model
**Contract:**
```rust
struct EvidenceBundle {
    query: String,
    results: Vec<SearchResult>,
    extracted_facts: Option<Vec<String>>,
    confidence: f32,
}

struct SearchResult {
    title: String,
    url: String,
    snippet: String,
}
```
**Value:** Freshness + citations as reusable primitive

#### 2. Memory Agent 🧠
**Capability:** Persistent learnings across sessions (Rust-native)
**API:**
```rust
recall(query: &str, tags: &[String], top_k: usize) -> Vec<MemoryEntry>
remember(text: String, tags: Vec<String>, confidence: f32, ttl: Option<Duration>)
forget(selector: &str)
```
**Storage:** `.zavora/memory.db` (SQLite)
**Store only:**
- User preferences ("use Nairobi timezone")
- Stable decisions ("chose MCP memory")
- Reusable facts ("repo layout")
**Value:** Orchestrator improves over time

#### 3. Time Agent ⏰
**Capability:** Time/date operations (Rust-native)
**Handshake at session start:**
```rust
struct TimeContext {
    now_iso: String,
    timezone: String,
    weekday: String,
    date: String,
}
```
**API:**
- `parse_relative("next Friday")` → DateTime
- `time_arithmetic(base, "+2 days")` → DateTime
- `normalize_timezone(time, from_tz, to_tz)` → DateTime
**Value:** Deterministic time reasoning, no scheduling bugs

### Tier 2: Workflow Agents (Execution Patterns)

#### 4. File Search Loop Agent 🔄
**Purpose:** Find all relevant files until coverage is good
**Loop:**
1. Propose search queries
2. Run searches (grep/find/ripgrep)
3. Cluster results
4. Stop when:
   - Saturation (no new unique results)
   - Confidence threshold reached
   - Max iterations hit

**Output:**
```rust
struct ResourceMap {
    key_files: Vec<(PathBuf, String)>, // path + why it matters
    gaps: Vec<String>, // "still missing X"
    coverage_score: f32,
}
```
**Value:** Comprehensive repo/docs discovery

#### 5. Sequential Execution Agent 📋
**Purpose:** Plan + execute with progress tracking
**Contract:**
```rust
make_plan(goal, constraints, resources, time_context) -> Plan
execute_step(step_id) -> StepResult

struct Plan {
    steps: Vec<Step>,
    acceptance_criteria: Vec<String>,
}

struct StepResult {
    step_id: usize,
    status: StepStatus,
    artifacts: Vec<Artifact>, // patch, file, command, summary
}
```
**Hard rule:** One step at a time, always produces artifacts
**Value:** Reliable machine vs wandering assistant

#### 6. Quality Agent ✅
**Purpose:** Verify outputs against acceptance criteria
**Inputs:**
- Artifacts
- Plan + progress
- Requirements/constraints

**Output:**
```rust
struct VerificationReport {
    pass: bool,
    issues: Vec<Issue>,
    suggested_fixes: HashMap<usize, Vec<String>>, // step_id -> fixes
    evidence_missing: Vec<String>,
}

struct Issue {
    severity: Severity,
    description: String,
    location: Option<String>,
}
```
**Hard rule:** Never generates new work, only evaluates + prescribes
**Value:** Creates trust through separation of concerns

## Orchestrator Loop

```
1. Bootstrap
   ├─ Time handshake → get current context
   └─ Memory recall → retrieve relevant learnings

2. Gather (if needed)
   ├─ Search agent → web research
   └─ File loop agent → codebase discovery

3. Plan
   └─ Sequential agent → make_plan() + acceptance criteria

4. Execute
   └─ Sequential agent → execute_step() with checklist updates

5. Verify
   └─ Quality agent → validate against criteria

6. Repair (if failed)
   └─ Push issues back into plan as new steps → goto Execute

7. Commit
   └─ Memory agent → store decisions/learnings
```

## System Prompt Structure

```rust
const ORCHESTRATOR_INSTRUCTION: &str = "
You are the orchestrator. You coordinate specialist agents:

CAPABILITY AGENTS (call when you need their unique skills):
- search_agent: Web search for current info (only if Gemini model)
- memory_agent: Recall/store persistent learnings
- time_agent: Time/date operations

WORKFLOW AGENTS (delegate complex multi-step work):
- file_search_loop_agent: Comprehensive file discovery
- sequential_execution_agent: Plan and execute with tracking
- quality_agent: Verify work against criteria

ORCHESTRATION LOOP:
1. Bootstrap: Get time context, recall relevant memories
2. Gather: Search/discover if needed
3. Plan: Create structured plan with acceptance criteria
4. Execute: Run steps one at a time
5. Verify: Check quality
6. Repair: Fix issues if verification fails
7. Commit: Store learnings

RULES:
- Always start with time handshake
- Memory agent never decides what to store (you decide)
- Sequential agent executes one step at a time
- Quality agent only evaluates, never generates work
- Store only high-signal learnings in memory
";
```

## Implementation Files

```
src/agents/
├── mod.rs              # Agent registry
├── memory.rs           # Native memory implementation
├── time.rs             # Native time implementation
├── search.rs           # Search agent (Gemini wrapper)
├── file_loop.rs        # File search loop agent
├── sequential.rs       # Sequential execution agent
├── quality.rs          # Quality verification agent
└── orchestrator.rs     # Main orchestrator logic
```

## Value Proposition

**Before:** Weak agents with filtered tools, no clear benefit
**After:** 
- 3 capability agents with unique skills (search, memory, time)
- 3 workflow agents with structured execution patterns
- Predictable orchestration loop
- Measurable improvement over single-agent

**Product story:** "Repeatable execution with verification"


## Implementation Status

### ✅ Phase 1: Complete (All Agents Implemented)

**Capability Agents:**
- ✅ Time Agent (`src/agents/time.rs`) - Native Rust implementation
- ✅ Memory Agent (`src/agents/memory.rs`) - Native Rust with JSON storage
- ✅ Search Agent (`src/agents/search.rs`) - Gemini Google Search wrapper

**Workflow Agents:**
- ✅ File Loop Agent (`src/agents/file_loop.rs`) - Iterative file discovery
- ✅ Sequential Agent (`src/agents/sequential.rs`) - Plan + execute with tracking
- ✅ Quality Agent (`src/agents/quality.rs`) - Verification against criteria

**Orchestrator:**
- ✅ Orchestrator (`src/agents/orchestrator.rs`) - Full execution loop

### 📋 Phase 2: Integration (Next Steps)

1. **Wire agents into runner** - Replace old git/research/planner agents
2. **Add agent tools** - Expose memory/time/search as tools to main agent
3. **Chat commands** - Add `/memory`, `/time`, `/orchestrate` commands
4. **System prompt** - Update orchestrator instruction
5. **Testing** - End-to-end orchestration tests

## Quick Start (When Integrated)

```bash
# Enable memory and time agents (always available)
zavora-cli chat

# Use search agent (requires Gemini)
zavora-cli --provider gemini --model gemini-2.0-flash-exp chat

# Run orchestrated execution
zavora-cli orchestrate "Implement feature X with tests"
```
