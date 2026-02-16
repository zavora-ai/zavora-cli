# Quick Start: Multi-Agent Features

## Memory Agent

### Store Learnings
```bash
$ zavora-cli chat
> /memory remember "I prefer dark mode for all UIs"
Stored: I prefer dark mode for all UIs

> /memory remember "Use TypeScript for new projects"
Stored: Use TypeScript for new projects
```

### Recall Learnings
```bash
> /memory recall preferences
Found 2 memories:
  [0.9] I prefer dark mode for all UIs
    tags: manual
  [0.9] Use TypeScript for new projects
    tags: manual
```

### LLM Uses Memory
```bash
> What are my coding preferences?
[Agent calls memory_agent tool to recall]

Based on your stored preferences:
- You prefer dark mode for all UIs
- You use TypeScript for new projects
```

### Forget Learnings
```bash
> /memory forget "dark mode"
Removed 1 memories
```

---

## Time Agent

### Current Time
```bash
> /time
Current time: 2026-02-16T20:10:00+03:00
Timezone: UTC
Weekday: Monday
Date: 2026-02-16
```

### Parse Relative Dates
```bash
> /time next Friday
next Friday → 2026-02-21T20:10:00+03:00

> /time in 3 days
in 3 days → 2026-02-19T20:10:00+03:00

> /time tomorrow
tomorrow → 2026-02-17T20:10:00+03:00
```

### LLM Uses Time
```bash
> Schedule a meeting for next Friday at 2pm
[Agent calls time_agent to parse "next Friday"]

I'll schedule the meeting for Friday, February 21, 2026 at 2:00 PM.

> What day is it in 5 days?
[Agent calls time_agent]

In 5 days it will be Saturday, February 21, 2026.
```

---

## Auto-Compaction

### Check Status
```bash
> /usage
Context  45000/128000 tokens (35%)

  User:      12000 tokens
  Assistant: 28000 tokens
  Tools:      3000 tokens
  System:     2000 tokens
```

### Toggle Auto-Compaction
```bash
> /autocompact
Auto-compaction disabled (threshold=75%, target=10%)

> /autocompact
Auto-compaction enabled (threshold=75%, target=10%)
```

### Manual Compaction
```bash
> /compact
Compacting conversation...
Compacted 15 events into ~2500 tokens. Kept 2 recent messages.
```

### How It Works
- Monitors token usage after each response
- Triggers at 75% context usage
- Uses LLM to create structured summary
- Compacts down to 10% usage
- Preserves recent messages

---

## Orchestration

### Run Full Loop
```bash
> /orchestrate Create a REST API with user authentication

Starting orchestration...

## Execution Result: ✓ PASSED

**Goal:** Create a REST API with user authentication

**Steps:** 5
**Artifacts:** 8
**Issues:** 0

### Plan
1. Design API endpoints
2. Implement authentication
3. Create user model
4. Add tests
5. Document API

### Artifacts
- File: src/api/auth.rs
- File: src/models/user.rs
- File: tests/auth_test.rs
- Summary: API implementation complete
```

### Orchestration Pattern
```
1. Bootstrap
   ├─ Time handshake (current context)
   └─ Memory recall (relevant learnings)

2. Gather
   ├─ Search agent (if needed)
   └─ File loop agent (discover resources)

3. Plan
   └─ Sequential agent creates structured plan

4. Execute
   └─ Sequential agent runs steps one-by-one

5. Verify
   └─ Quality agent checks against criteria

6. Repair (if needed)
   └─ Fix issues and re-verify

7. Commit
   └─ Memory agent stores learnings
```

---

## Configuration

### Enable Features
```toml
# .zavora/config.toml
[profiles.default]
provider = "openai"
model = "gpt-4.1"

# Auto-compaction (default: enabled)
auto_compact_enabled = true
compaction_threshold = 0.75  # Trigger at 75%
compaction_target = 0.10     # Compact to 10%

# Session persistence
session_backend = "sqlite"
session_db_url = "sqlite://.zavora/sessions.db"
```

### Memory Storage
```bash
# Stored in workspace
.zavora/
├── memory.json      # Persistent learnings
├── sessions.db      # Conversation history
└── todos/          # Task lists
```

---

## Tips

### Memory Best Practices
- Store high-signal information only
- Use descriptive text
- Add relevant tags
- Set confidence scores

```bash
> /memory remember "Project uses React 18 with TypeScript" 
# Good: Specific, actionable

> /memory remember "I like React"
# Bad: Too vague
```

### Time Queries
Supported formats:
- `now`, `today`, `tomorrow`, `yesterday`
- `next Friday`, `next Monday`
- `in 2 days`, `in 3 hours`, `in 1 week`

### Orchestration
Best for:
- Multi-step implementations
- Features requiring tests
- Complex refactoring
- Structured planning

Not needed for:
- Simple questions
- Single file changes
- Quick fixes

---

## Troubleshooting

### Memory Not Persisting
```bash
# Check file exists
ls .zavora/memory.json

# Check permissions
chmod 644 .zavora/memory.json
```

### Time Parsing Fails
```bash
# Use simpler expressions
> /time next Friday  # ✓ Works
> /time Friday       # ✗ Ambiguous
```

### Compaction Issues
```bash
# Check usage first
> /usage

# Manual compact if needed
> /compact

# Disable auto-compact if problematic
> /autocompact
```

---

## Examples

### Complete Workflow
```bash
# 1. Store preference
> /memory remember "Use Rust for CLI tools"

# 2. Check time
> /time
Current time: 2026-02-16T20:10:00+03:00

# 3. Orchestrate work
> /orchestrate Build a CLI tool for file processing

# 4. Agent uses memory
[Recalls: "Use Rust for CLI tools"]
[Creates Rust project]

# 5. Store learning
[Commits: "Built CLI tool with clap + tokio"]

# 6. Check usage
> /usage
Context  95000/128000 tokens (74%)

# 7. Auto-compact triggers at 75%
[Compacts to 12800 tokens (10%)]
```

---

## Learn More

- Architecture: `docs/MULTIAGENT_ARCHITECTURE.md`
- Integration: `docs/MULTIAGENT_INTEGRATION.md`
- Session Summary: `SESSION_SUMMARY.md`
- Main README: `README.md`
