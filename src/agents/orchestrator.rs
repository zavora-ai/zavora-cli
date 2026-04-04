/// Orchestrator - Coordinates capability and workflow agents.
use anyhow::Result;
use serde::{Deserialize, Serialize};

use super::file_loop::{FileLoopAgent, FileLoopConfig, ResourceMap};
use super::quality::{QualityAgent, VerificationReport};
use super::sequential::{Artifact, Plan, SequentialAgent};
use super::time::{TimeAgent, TimeContext};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OrchestratorConfig {
    pub max_repair_iterations: usize,
    pub enable_search: bool,
    pub enable_memory: bool,
}

impl Default for OrchestratorConfig {
    fn default() -> Self {
        Self {
            max_repair_iterations: 3,
            enable_search: true,
            enable_memory: true,
        }
    }
}

pub struct Orchestrator {
    config: OrchestratorConfig,
    sequential: SequentialAgent,
    file_loop: FileLoopAgent,
}

impl Orchestrator {
    pub fn new(config: OrchestratorConfig) -> Self {
        Self {
            config,
            sequential: SequentialAgent::new(),
            file_loop: FileLoopAgent::new(FileLoopConfig::default()),
        }
    }

    /// Execute the full orchestration loop.
    pub async fn execute(
        &mut self,
        goal: String,
        requirements: Vec<String>,
    ) -> Result<ExecutionResult> {
        // 1. Bootstrap: Time handshake + Memory recall
        let time_context = TimeAgent::handshake();
        let _memories = super::memory::recall(&goal, 5).await.unwrap_or_default();

        // 2. Gather: File discovery (if needed)
        let resources = if goal.contains("file") || goal.contains("code") {
            Some(self.file_loop.search_loop(&goal).await?)
        } else {
            None
        };

        // 3. Plan: Create structured plan
        let resource_paths = resources
            .as_ref()
            .map(|r| {
                r.key_files
                    .iter()
                    .map(|(p, _)| p.display().to_string())
                    .collect()
            })
            .unwrap_or_default();

        let plan = self.sequential.make_plan(
            goal.clone(),
            requirements.clone(),
            resource_paths,
            &time_context.now_iso,
        )?;

        // 4. Execute: Run steps one at a time
        let mut step_results = Vec::new();
        for step in &plan.steps {
            let result = self.sequential.execute_step(step.id).await?;
            step_results.push(result);
        }

        // 5. Verify: Check quality
        let artifacts = self.sequential.get_artifacts();
        let verification = QualityAgent::verify(&artifacts, &plan, &requirements)?;

        // 6. Repair loop: Fix issues if verification failed
        let repair_iteration = 0;
        let final_verification =
            if !verification.pass && repair_iteration < self.config.max_repair_iterations {
                // Create repair steps from issues
                // Execute repair steps
                // Re-verify
                // For now, return original verification
                verification.clone()
            } else {
                verification
            };

        // 7. Commit: Store learnings in memory
        if final_verification.pass {
            let _ = super::memory::remember(
                &format!("Successfully completed: {}", goal),
            ).await;
        }
        if final_verification.pass && plan.steps.len() > 2 {
            let _ = super::memory::remember(
                &format!("Effective plan for '{}': {} steps", goal, plan.steps.len()),
            ).await;
        }

        Ok(ExecutionResult {
            goal,
            plan,
            artifacts,
            verification: final_verification,
            time_context,
            resources,
        })
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExecutionResult {
    pub goal: String,
    pub plan: Plan,
    pub artifacts: Vec<Artifact>,
    pub verification: VerificationReport,
    pub time_context: TimeContext,
    pub resources: Option<ResourceMap>,
}

impl ExecutionResult {
    pub fn format_summary(&self) -> String {
        let status = if self.verification.pass {
            "✓ PASSED"
        } else {
            "✗ FAILED"
        };

        let mut summary = format!("## Execution Result: {}\n\n", status);
        summary.push_str(&format!("**Goal:** {}\n\n", self.goal));
        summary.push_str(&format!("**Steps:** {}\n", self.plan.steps.len()));
        summary.push_str(&format!("**Artifacts:** {}\n", self.artifacts.len()));
        summary.push_str(&format!(
            "**Issues:** {}\n\n",
            self.verification.issues.len()
        ));

        if !self.verification.issues.is_empty() {
            summary.push_str("### Issues\n");
            for issue in &self.verification.issues {
                summary.push_str(&format!("- [{:?}] {}\n", issue.severity, issue.description));
            }
        }

        summary
    }
}
