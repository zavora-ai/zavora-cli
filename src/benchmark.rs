/// Parity benchmark suite for measuring coding outcomes against reference CLI.
///
/// Defines benchmark scenarios, a scoring rubric, and a scorecard for tracking
/// parity across project creation, file edits, and GitHub workflows.
use serde::{Deserialize, Serialize};

// ---------------------------------------------------------------------------
// Scenario definitions
// ---------------------------------------------------------------------------

/// A benchmark scenario category.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum BenchmarkCategory {
    ProjectCreation,
    FileEdits,
    GitHubWorkflows,
    ChatUX,
    ToolExecution,
    ContextManagement,
}

impl BenchmarkCategory {
    pub fn label(&self) -> &'static str {
        match self {
            Self::ProjectCreation => "Project Creation",
            Self::FileEdits => "File Edits",
            Self::GitHubWorkflows => "GitHub Workflows",
            Self::ChatUX => "Chat UX",
            Self::ToolExecution => "Tool Execution",
            Self::ContextManagement => "Context Management",
        }
    }
}

/// A single benchmark scenario with expected outcome.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BenchmarkScenario {
    pub id: String,
    pub category: BenchmarkCategory,
    pub description: String,
    pub weight: f64,
    pub pass_criteria: String,
}

/// Result of evaluating a single scenario.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ScenarioResult {
    pub scenario_id: String,
    pub passed: bool,
    pub score: f64,
    pub notes: String,
}

// ---------------------------------------------------------------------------
// Scoring rubric
// ---------------------------------------------------------------------------

/// Score levels for the rubric.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ParityLevel {
    /// Feature fully matches or exceeds reference.
    Met,
    /// Feature partially implemented.
    Partial,
    /// Feature missing or non-functional.
    NotMet,
}

impl ParityLevel {
    pub fn label(&self) -> &'static str {
        match self {
            Self::Met => "✓ Met",
            Self::Partial => "◐ Partial",
            Self::NotMet => "✗ Not Met",
        }
    }

    pub fn score(&self) -> f64 {
        match self {
            Self::Met => 1.0,
            Self::Partial => 0.5,
            Self::NotMet => 0.0,
        }
    }
}

// ---------------------------------------------------------------------------
// Scorecard
// ---------------------------------------------------------------------------

/// Aggregate scorecard across all benchmark scenarios.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Scorecard {
    pub results: Vec<ScenarioResult>,
    pub total_score: f64,
    pub max_score: f64,
    pub pass_rate: f64,
}

impl Scorecard {
    /// Compute a scorecard from scenario results and their weights.
    pub fn compute(scenarios: &[BenchmarkScenario], results: &[ScenarioResult]) -> Self {
        let mut total_score = 0.0;
        let mut max_score = 0.0;
        for scenario in scenarios {
            max_score += scenario.weight;
            if let Some(result) = results.iter().find(|r| r.scenario_id == scenario.id) {
                total_score += result.score * scenario.weight;
            }
        }
        let pass_rate = if max_score > 0.0 {
            total_score / max_score
        } else {
            0.0
        };
        Scorecard {
            results: results.to_vec(),
            total_score,
            max_score,
            pass_rate,
        }
    }

    /// Format the scorecard for display.
    pub fn format_display(&self) -> String {
        let mut out = format!(
            "Parity Scorecard: {:.1}/{:.1} ({:.0}%)\n",
            self.total_score,
            self.max_score,
            self.pass_rate * 100.0
        );
        for result in &self.results {
            let status = if result.passed { "✓" } else { "✗" };
            out.push_str(&format!(
                "  [{status}] {} (score: {:.1}) {}\n",
                result.scenario_id, result.score, result.notes
            ));
        }
        out
    }

    /// Check if the scorecard meets a minimum pass rate threshold.
    pub fn meets_threshold(&self, threshold: f64) -> bool {
        self.pass_rate >= threshold
    }
}

// ---------------------------------------------------------------------------
// Default scenario catalog
// ---------------------------------------------------------------------------

/// Build the default parity benchmark scenario catalog.
pub fn default_scenarios() -> Vec<BenchmarkScenario> {
    vec![
        BenchmarkScenario {
            id: "pc-01".into(),
            category: BenchmarkCategory::ProjectCreation,
            description: "Create a new Rust project with Cargo.toml and src/main.rs".into(),
            weight: 1.0,
            pass_criteria: "Project compiles with cargo check".into(),
        },
        BenchmarkScenario {
            id: "pc-02".into(),
            category: BenchmarkCategory::ProjectCreation,
            description: "Scaffold project with README, .gitignore, and CI config".into(),
            weight: 1.0,
            pass_criteria: "All expected files present".into(),
        },
        BenchmarkScenario {
            id: "fe-01".into(),
            category: BenchmarkCategory::FileEdits,
            description: "Add a function to an existing file via fs_write patch".into(),
            weight: 1.0,
            pass_criteria: "File compiles after edit".into(),
        },
        BenchmarkScenario {
            id: "fe-02".into(),
            category: BenchmarkCategory::FileEdits,
            description: "Multi-file refactor across 3+ files".into(),
            weight: 1.5,
            pass_criteria: "All files compile, tests pass".into(),
        },
        BenchmarkScenario {
            id: "gh-01".into(),
            category: BenchmarkCategory::GitHubWorkflows,
            description: "Create a GitHub issue with labels".into(),
            weight: 1.0,
            pass_criteria: "Issue created with correct title and labels".into(),
        },
        BenchmarkScenario {
            id: "gh-02".into(),
            category: BenchmarkCategory::GitHubWorkflows,
            description: "Create a PR and update project board".into(),
            weight: 1.0,
            pass_criteria: "PR created, project item moved".into(),
        },
        BenchmarkScenario {
            id: "cx-01".into(),
            category: BenchmarkCategory::ChatUX,
            description: "Slash command discovery and fuzzy matching".into(),
            weight: 0.5,
            pass_criteria: "Unknown commands suggest alternatives".into(),
        },
        BenchmarkScenario {
            id: "cx-02".into(),
            category: BenchmarkCategory::ChatUX,
            description: "Context usage display and budget warnings".into(),
            weight: 0.5,
            pass_criteria: "/usage shows token breakdown".into(),
        },
        BenchmarkScenario {
            id: "te-01".into(),
            category: BenchmarkCategory::ToolExecution,
            description: "Execute bash with timeout and retry".into(),
            weight: 1.0,
            pass_criteria: "Command executes within timeout, retries on failure".into(),
        },
        BenchmarkScenario {
            id: "te-02".into(),
            category: BenchmarkCategory::ToolExecution,
            description: "MCP tool discovery with diagnostics".into(),
            weight: 1.0,
            pass_criteria: "MCP servers diagnosed with state and latency".into(),
        },
        BenchmarkScenario {
            id: "cm-01".into(),
            category: BenchmarkCategory::ContextManagement,
            description: "Manual compaction preserves recent context".into(),
            weight: 1.0,
            pass_criteria: "/compact reduces events, keeps recent messages".into(),
        },
        BenchmarkScenario {
            id: "cm-02".into(),
            category: BenchmarkCategory::ContextManagement,
            description: "Checkpoint save and restore integrity".into(),
            weight: 1.0,
            pass_criteria: "Restored session matches checkpoint state".into(),
        },
    ]
}

/// Baseline threshold for release readiness.
pub const BASELINE_THRESHOLD: f64 = 0.75;
/// Target threshold for full parity.
pub const TARGET_THRESHOLD: f64 = 0.90;
