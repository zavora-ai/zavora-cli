/// Quality agent - Verify outputs against acceptance criteria.
use anyhow::Result;
use serde::{Deserialize, Serialize};

use super::sequential::{Artifact, Plan};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VerificationReport {
    pub pass: bool,
    pub issues: Vec<Issue>,
    pub suggested_fixes: std::collections::HashMap<usize, Vec<String>>, // step_id -> fixes
    pub evidence_missing: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Issue {
    pub severity: Severity,
    pub description: String,
    pub location: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Severity {
    Critical,
    Major,
    Minor,
    Info,
}

pub struct QualityAgent;

impl QualityAgent {
    /// Verify artifacts against plan and acceptance criteria.
    pub fn verify(
        artifacts: &[Artifact],
        plan: &Plan,
        requirements: &[String],
    ) -> Result<VerificationReport> {
        let mut issues = Vec::new();
        let suggested_fixes = std::collections::HashMap::new();
        let mut evidence_missing = Vec::new();

        // Check if all acceptance criteria are met
        for criterion in &plan.acceptance_criteria {
            if !Self::check_criterion(criterion, artifacts) {
                issues.push(Issue {
                    severity: Severity::Major,
                    description: format!("Acceptance criterion not met: {}", criterion),
                    location: None,
                });
            }
        }

        // Check for common issues
        Self::check_code_quality(artifacts, &mut issues);
        Self::check_documentation(artifacts, &mut issues);
        Self::check_tests(artifacts, &mut issues);

        // Identify missing evidence
        for requirement in requirements {
            if !Self::has_evidence(requirement, artifacts) {
                evidence_missing.push(requirement.clone());
            }
        }

        let pass = issues.is_empty() && evidence_missing.is_empty();

        Ok(VerificationReport {
            pass,
            issues,
            suggested_fixes,
            evidence_missing,
        })
    }

    fn check_criterion(criterion: &str, artifacts: &[Artifact]) -> bool {
        // Placeholder: would analyze artifacts against criterion
        // For now, simple keyword matching
        let criterion_lower = criterion.to_lowercase();
        artifacts.iter().any(|artifact| match artifact {
            Artifact::Summary { text } => text.to_lowercase().contains(&criterion_lower),
            Artifact::File { content, .. } => content.to_lowercase().contains(&criterion_lower),
            _ => false,
        })
    }

    fn check_code_quality(artifacts: &[Artifact], issues: &mut Vec<Issue>) {
        for artifact in artifacts {
            if let Artifact::File { path, content } = artifact {
                // Check for basic quality issues
                if content.contains("TODO") || content.contains("FIXME") {
                    issues.push(Issue {
                        severity: Severity::Minor,
                        description: "Code contains TODO/FIXME markers".to_string(),
                        location: Some(path.clone()),
                    });
                }

                if content.lines().any(|line| line.len() > 120) {
                    issues.push(Issue {
                        severity: Severity::Info,
                        description: "Lines exceed 120 characters".to_string(),
                        location: Some(path.clone()),
                    });
                }
            }
        }
    }

    fn check_documentation(artifacts: &[Artifact], issues: &mut Vec<Issue>) {
        let has_docs = artifacts.iter().any(|a| match a {
            Artifact::File { path, .. } => {
                path.ends_with(".md") || path.contains("README") || path.contains("doc")
            }
            _ => false,
        });

        if !has_docs {
            issues.push(Issue {
                severity: Severity::Major,
                description: "No documentation found".to_string(),
                location: None,
            });
        }
    }

    fn check_tests(artifacts: &[Artifact], issues: &mut Vec<Issue>) {
        let has_tests = artifacts.iter().any(|a| match a {
            Artifact::File { path, .. } => path.contains("test"),
            Artifact::Command { cmd, .. } => cmd.contains("test"),
            _ => false,
        });

        if !has_tests {
            issues.push(Issue {
                severity: Severity::Critical,
                description: "No tests found or executed".to_string(),
                location: None,
            });
        }
    }

    fn has_evidence(requirement: &str, artifacts: &[Artifact]) -> bool {
        // Check if artifacts provide evidence for requirement
        artifacts.iter().any(|artifact| match artifact {
            Artifact::Summary { text } => text.contains(requirement),
            Artifact::File { content, .. } => content.contains(requirement),
            Artifact::Command { output, .. } => output.contains(requirement),
            _ => false,
        })
    }
}
