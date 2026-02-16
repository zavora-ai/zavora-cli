/// Sequential execution agent - Plan + execute with progress tracking.
use anyhow::Result;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Plan {
    pub goal: String,
    pub steps: Vec<Step>,
    pub acceptance_criteria: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Step {
    pub id: usize,
    pub description: String,
    pub dependencies: Vec<usize>,
    pub status: StepStatus,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum StepStatus {
    Pending,
    InProgress,
    Completed,
    Failed,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StepResult {
    pub step_id: usize,
    pub status: StepStatus,
    pub artifacts: Vec<Artifact>,
    pub error: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum Artifact {
    File { path: String, content: String },
    Patch { path: String, diff: String },
    Command { cmd: String, output: String },
    Summary { text: String },
}

pub struct SequentialAgent {
    current_plan: Option<Plan>,
    step_results: HashMap<usize, StepResult>,
}

impl SequentialAgent {
    pub fn new() -> Self {
        Self {
            current_plan: None,
            step_results: HashMap::new(),
        }
    }

    /// Create a plan from goal and constraints.
    pub fn make_plan(
        &mut self,
        goal: String,
        _constraints: Vec<String>,
        _resources: Vec<String>,
        _time_context: &str,
    ) -> Result<Plan> {
        // Placeholder: would use LLM to generate plan
        // For now, create simple plan structure
        let plan = Plan {
            goal: goal.clone(),
            steps: vec![
                Step {
                    id: 0,
                    description: format!("Analyze requirements for: {}", goal),
                    dependencies: vec![],
                    status: StepStatus::Pending,
                },
                Step {
                    id: 1,
                    description: "Implement solution".to_string(),
                    dependencies: vec![0],
                    status: StepStatus::Pending,
                },
                Step {
                    id: 2,
                    description: "Verify implementation".to_string(),
                    dependencies: vec![1],
                    status: StepStatus::Pending,
                },
            ],
            acceptance_criteria: vec![
                "Solution meets requirements".to_string(),
                "All tests pass".to_string(),
                "Code is documented".to_string(),
            ],
        };

        self.current_plan = Some(plan.clone());
        Ok(plan)
    }

    /// Execute a single step.
    pub async fn execute_step(&mut self, step_id: usize) -> Result<StepResult> {
        let plan = self
            .current_plan
            .as_mut()
            .ok_or_else(|| anyhow::anyhow!("No active plan"))?;

        let step = plan
            .steps
            .iter_mut()
            .find(|s| s.id == step_id)
            .ok_or_else(|| anyhow::anyhow!("Step {} not found", step_id))?;

        // Check dependencies
        for dep_id in &step.dependencies {
            if let Some(dep_result) = self.step_results.get(dep_id) {
                if dep_result.status != StepStatus::Completed {
                    return Err(anyhow::anyhow!(
                        "Dependency step {} not completed",
                        dep_id
                    ));
                }
            } else {
                return Err(anyhow::anyhow!("Dependency step {} not executed", dep_id));
            }
        }

        step.status = StepStatus::InProgress;

        // Placeholder: would execute actual work here
        // For now, mark as completed with dummy artifact
        let result = StepResult {
            step_id,
            status: StepStatus::Completed,
            artifacts: vec![Artifact::Summary {
                text: format!("Completed: {}", step.description),
            }],
            error: None,
        };

        step.status = result.status;
        self.step_results.insert(step_id, result.clone());

        Ok(result)
    }

    /// Get current progress.
    pub fn get_progress(&self) -> Option<(usize, usize)> {
        self.current_plan.as_ref().map(|plan| {
            let completed = plan
                .steps
                .iter()
                .filter(|s| s.status == StepStatus::Completed)
                .count();
            (completed, plan.steps.len())
        })
    }

    /// Get all artifacts produced so far.
    pub fn get_artifacts(&self) -> Vec<Artifact> {
        self.step_results
            .values()
            .flat_map(|r| r.artifacts.clone())
            .collect()
    }
}

impl Default for SequentialAgent {
    fn default() -> Self {
        Self::new()
    }
}
