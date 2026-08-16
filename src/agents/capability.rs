use std::sync::Arc;

use adk_rust::ToolExecutionStrategy;
use adk_rust::prelude::*;
use anyhow::Result;

use crate::capabilities::{CapabilityCategory, CapabilityToolset};

struct SpecialistSpec {
    name: &'static str,
    description: &'static str,
    instruction: &'static str,
    category: CapabilityCategory,
}

const SPECIALISTS: &[SpecialistSpec] = &[
    SpecialistSpec {
        name: "artifact_agent",
        description: "Creates and edits documents, presentations, spreadsheets, PDFs, email, and other work artifacts",
        instruction: "You are Zavora's artifact specialist. Produce the requested work artifact using the available productivity tools. Inspect existing files before editing, preserve formatting and user content, and verify generated output before returning a concise result. Treat sending email or publishing content as an external write that requires approval.",
        category: CapabilityCategory::Productivity,
    },
    SpecialistSpec {
        name: "developer_agent",
        description: "Handles repositories, code search, implementation, tests, dependencies, CI/CD, and delivery",
        instruction: "You are Zavora's development specialist. Inspect the repository, make the smallest correct change, follow its conventions, and run proportionate verification. Never claim a change succeeded without tool evidence. Treat deployments, destructive git operations, and production changes as consequential actions.",
        category: CapabilityCategory::Development,
    },
    SpecialistSpec {
        name: "research_agent",
        description: "Performs source-grounded web, news, market, legal, medical, and domain research",
        instruction: "You are Zavora's research specialist. Prefer primary and current sources, distinguish evidence from inference, preserve source URLs, and report uncertainty. Research tools are read-only by default. Do not turn medical, legal, or financial evidence into an unsupported professional decision.",
        category: CapabilityCategory::Research,
    },
    SpecialistSpec {
        name: "operations_agent",
        description: "Handles device health, desktop automation, infrastructure, incidents, and business operations",
        instruction: "You are Zavora's operations specialist. Diagnose before changing state, identify the exact target, prefer read-only checks, and require approval for remediations, process termination, package changes, service restarts, device commands, and production operations. Return evidence and rollback guidance.",
        category: CapabilityCategory::Operations,
    },
    SpecialistSpec {
        name: "reviewer_agent",
        description: "Reviews outputs for correctness, safety, provenance, governance, and acceptance criteria",
        instruction: "You are Zavora's independent reviewer. Verify the result against the user's requirements and available evidence. Identify missing tests, unsafe writes, weak provenance, policy violations, and incomplete acceptance criteria. Do not perform consequential actions; return a clear pass/fail assessment with actionable findings.",
        category: CapabilityCategory::Platform,
    },
];

pub fn specialist_names() -> impl Iterator<Item = &'static str> {
    SPECIALISTS.iter().map(|spec| spec.name)
}

pub fn specialist_description(name: &str) -> Option<&'static str> {
    SPECIALISTS
        .iter()
        .find(|spec| spec.name == name)
        .map(|spec| spec.description)
}

pub fn build_specialist_agents(
    model: Arc<dyn Llm>,
    tools: &[Arc<dyn Tool>],
) -> Result<Vec<Arc<dyn Agent>>> {
    let workspace = crate::skills::resolve_workspace_instructions()
        .ok()
        .filter(|instructions| !instructions.content.is_empty())
        .map(|instructions| {
            format!(
                "\n\n<workspace_instructions>\n{}\n</workspace_instructions>",
                instructions.content
            )
        })
        .unwrap_or_default();
    SPECIALISTS
        .iter()
        .map(|spec| {
            let toolset = CapabilityToolset::specialist(
                format!("{}-capabilities", spec.name),
                spec.category,
                tools.to_vec(),
            );
            let agent = LlmAgentBuilder::new(spec.name)
                .description(spec.description)
                .instruction(format!("{}{}", spec.instruction, workspace))
                .model(model.clone())
                .toolset(Arc::new(toolset))
                .tool_execution_strategy(ToolExecutionStrategy::Auto)
                .build()?;
            Ok(Arc::new(agent) as Arc<dyn Agent>)
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn specialist_names_are_unique() {
        let names = specialist_names().collect::<std::collections::BTreeSet<_>>();
        assert_eq!(names.len(), SPECIALISTS.len());
    }
}
