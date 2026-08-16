# Parity Benchmark Suite

Objective measurement of zavora-cli coding outcomes against Q CLI reference capabilities.

## Scoring Rubric

| Level | Score | Meaning |
|-------|-------|---------|
| ✓ Met | 1.0 | Feature fully matches or exceeds reference |
| ◐ Partial | 0.5 | Feature partially implemented |
| ✗ Not Met | 0.0 | Feature missing or non-functional |

## Thresholds

- **Baseline** (release readiness): 75%
- **Target** (full parity): 90%

## Benchmark Scenarios

### Project Creation (weight: 1.0 each)

| ID | Scenario | Pass Criteria |
|----|----------|---------------|
| pc-01 | Create Rust project with Cargo.toml and src/main.rs | Project compiles with cargo check |
| pc-02 | Scaffold project with README, .gitignore, CI config | All expected files present |

### File Edits (weight: 1.0–1.5)

| ID | Scenario | Pass Criteria |
|----|----------|---------------|
| fe-01 | Add function to existing file via fs_write patch | File compiles after edit |
| fe-02 | Multi-file refactor across 3+ files (weight: 1.5) | All files compile, tests pass |

### GitHub Workflows (weight: 1.0 each)

| ID | Scenario | Pass Criteria |
|----|----------|---------------|
| gh-01 | Create GitHub issue with labels | Issue created with correct title and labels |
| gh-02 | Create PR and update project board | PR created, project item moved |

### Chat UX (weight: 0.5 each)

| ID | Scenario | Pass Criteria |
|----|----------|---------------|
| cx-01 | Slash command discovery and fuzzy matching | Unknown commands suggest alternatives |
| cx-02 | Context usage display and budget warnings | /usage shows token breakdown |

### Tool Execution (weight: 1.0 each)

| ID | Scenario | Pass Criteria |
|----|----------|---------------|
| te-01 | Execute bash with timeout and retry | Command executes within timeout |
| te-02 | MCP tool discovery with diagnostics | Servers diagnosed with state and latency |

### Context Management (weight: 1.0 each)

| ID | Scenario | Pass Criteria |
|----|----------|---------------|
| cm-01 | Manual compaction preserves recent context | /compact reduces events, keeps recent |
| cm-02 | Checkpoint save and restore integrity | Restored session matches checkpoint |

## Total Weight: 11.5

## Running Benchmarks

```bash
# Run the parity benchmark check
make parity-check
```

The benchmark harness is in `src/benchmark.rs`. Scorecard output shows per-scenario pass/fail and aggregate score.

## Integration

Benchmark checks are integrated into the release readiness workflow:
- `make release-check` includes parity threshold validation
- Scorecard is published as part of release artifacts
- Regressions below baseline threshold block release
