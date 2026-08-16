//! Keeping the system prompt honest about what the runtime actually offers.
//!
//! The v2 orchestrator prompt advertised `file_search_agent`, `sequential_agent`
//! and `quality_agent`, and instructed the model to use `sequential_agent` for
//! complex work. None of the three was ever registered. Every session paid
//! tokens to describe tools that could only fail when called, because nothing
//! connected the prompt text to the registry.
//!
//! [`PromptSurface`] closes that loop: it enumerates what is registered, renders
//! the prompt's capability and agent sections from that enumeration, and can
//! audit any prompt for names the runtime cannot serve.
//!
//! Requirements 6.2, 6.5, 6.8, 13.3; Correctness Property 3.

use std::collections::BTreeSet;

use crate::tool_surface::ResolvedRuntimeTools;

/// A name a prompt referred to that the runtime does not provide.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub struct PhantomReference {
    /// The advertised name.
    pub name: String,
    /// The prompt line it appeared on, for a comprehensible failure message.
    pub line: String,
}

/// What the runtime can actually serve on this turn.
#[derive(Debug, Clone, Default)]
pub struct PromptSurface {
    registered: BTreeSet<String>,
}

impl PromptSurface {
    /// Build from the sealed tool surface plus any agent names registered as
    /// sub-agents rather than as tools.
    pub fn new(tools: &ResolvedRuntimeTools, agent_names: &[&str]) -> Self {
        let mut registered = tools.names();
        registered.extend(agent_names.iter().map(|name| (*name).to_string()));
        Self { registered }
    }

    /// Build from an explicit name set. Used by callers that have not sealed a
    /// surface yet, and by tests.
    pub fn from_names<I, S>(names: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        Self {
            registered: names.into_iter().map(Into::into).collect(),
        }
    }

    pub fn contains(&self, name: &str) -> bool {
        self.registered.contains(name)
    }

    pub fn names(&self) -> &BTreeSet<String> {
        &self.registered
    }

    /// Render a `- name: description` bullet list for the names that are
    /// actually registered, preserving the caller's ordering and dropping the
    /// rest.
    ///
    /// This is the generative half of the fix: a section built this way cannot
    /// name a phantom, because a name that is not registered is not emitted.
    pub fn render_section(&self, entries: &[(&str, &str)]) -> String {
        entries
            .iter()
            .filter(|(name, _)| self.contains(name))
            .map(|(name, description)| format!("- {name}: {description}\n"))
            .collect()
    }

    /// Audit a composed prompt for advertised names the runtime cannot serve.
    ///
    /// Only `- <name>: <description>` bullets are considered, which is the shape
    /// the prompt uses to enumerate callable things. Prose is left alone: it
    /// discusses concepts, not call targets.
    pub fn audit(&self, prompt: &str) -> Vec<PhantomReference> {
        let mut phantoms = Vec::new();

        for line in prompt.lines() {
            let trimmed = line.trim();
            let Some(rest) = trimmed.strip_prefix("- ") else {
                continue;
            };
            let Some((name, _)) = rest.split_once(':') else {
                continue;
            };
            let name = name.trim();

            // Only names shaped like a registered identifier are call targets.
            // Every tool and agent in this runtime is lowercase snake_case, so
            // requiring that keeps documentary bullets out of the audit.
            //
            // This matters: the system prompt carries a `<system_context>` block
            // with `- Shell: {shell}`, which an identifier check that allowed
            // capitals would classify as a phantom tool named "Shell" and strip,
            // silently removing the shell from the model's context.
            let is_identifier = !name.is_empty()
                && name
                    .chars()
                    .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_' || c == '-');
            if !is_identifier {
                continue;
            }

            if !self.contains(name) {
                phantoms.push(PhantomReference {
                    name: name.to_string(),
                    line: trimmed.to_string(),
                });
            }
        }

        phantoms
    }

    /// Audit and fail with a comprehensible message.
    pub fn assert_no_phantoms(&self, prompt: &str) -> Result<(), Vec<PhantomReference>> {
        let phantoms = self.audit(prompt);
        if phantoms.is_empty() {
            Ok(())
        } else {
            Err(phantoms)
        }
    }

    /// Remove advertised names the runtime cannot serve.
    ///
    /// Belt to `render_section`'s braces. Generation prevents a phantom from
    /// being written; sanitizing prevents one that slipped into hand-written
    /// prose from reaching the model, which is what actually costs tokens and
    /// produces failed tool calls. Returns the cleaned prompt and what was
    /// removed, so the caller can report it rather than hide it.
    pub fn sanitize(&self, prompt: &str) -> (String, Vec<PhantomReference>) {
        let phantoms = self.audit(prompt);
        if phantoms.is_empty() {
            return (prompt.to_string(), phantoms);
        }

        let removed = phantoms
            .iter()
            .map(|phantom| phantom.line.as_str())
            .collect::<BTreeSet<_>>();

        let cleaned = prompt
            .lines()
            .filter(|line| !removed.contains(line.trim()))
            .collect::<Vec<_>>()
            .join("\n");

        (cleaned, phantoms)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_registered_name_is_not_a_phantom() {
        let surface = PromptSurface::from_names(["fs_read", "memory_agent"]);
        let prompt = "\
Tools:
- fs_read: Read a file
- memory_agent: Recall user preferences
";
        assert_eq!(surface.audit(prompt), vec![]);
    }

    #[test]
    fn an_unregistered_name_is_reported() {
        let surface = PromptSurface::from_names(["fs_read"]);
        let prompt = "\
- fs_read: Read a file
- sequential_agent: Create plans and execute steps
";
        let phantoms = surface.audit(prompt);
        assert_eq!(phantoms.len(), 1, "{phantoms:?}");
        assert_eq!(phantoms[0].name, "sequential_agent");
    }

    #[test]
    fn prose_bullets_are_not_audited() {
        let surface = PromptSurface::from_names(["search_agent"]);
        let prompt = "\
RULES:
- For news/web searches: delegate to search_agent
- Store only high-signal learnings: user preferences, decisions, patterns
";
        assert_eq!(surface.audit(prompt), vec![]);
    }

    /// Regression: the `<system_context>` block uses `- Shell: {shell}`, which an
    /// identifier check allowing capitals classified as a phantom tool and
    /// stripped, silently removing the shell from the model's context.
    #[test]
    fn capitalized_documentary_bullets_are_not_phantoms() {
        let surface = PromptSurface::from_names(["fs_read"]);
        let prompt = "\
<system_context>
- Operating System: macos
- Current Directory: /tmp
- Shell: /bin/zsh
</system_context>
";
        assert_eq!(
            surface.audit(prompt),
            vec![],
            "documentary bullets were audited"
        );
        let (sanitized, phantoms) = surface.sanitize(prompt);
        assert!(phantoms.is_empty());
        assert!(
            sanitized.contains("- Shell: /bin/zsh"),
            "the shell line was stripped from the prompt: {sanitized}"
        );
    }

    /// A lowercase snake_case name is still audited, because that is what every
    /// registered tool and agent actually looks like.
    #[test]
    fn lowercase_identifiers_are_still_audited() {
        let surface = PromptSurface::from_names(["fs_read"]);
        let phantoms = surface.audit("- quality_agent: Verify work\n");
        assert_eq!(phantoms.len(), 1);
        assert_eq!(phantoms[0].name, "quality_agent");
    }

    #[test]
    fn rendering_omits_unregistered_names() {
        let surface = PromptSurface::from_names(["time_agent"]);
        let rendered = surface.render_section(&[
            ("time_agent", "Get current time"),
            ("quality_agent", "Verify work against acceptance criteria"),
        ]);
        assert_eq!(rendered, "- time_agent: Get current time\n");
        assert_eq!(surface.audit(&rendered), vec![]);
    }

    #[test]
    fn assert_no_phantoms_reports_every_offender() {
        let surface = PromptSurface::from_names(["fs_read"]);
        let prompt = "- a_agent: one\n- b_agent: two\n";
        let error = surface.assert_no_phantoms(prompt).unwrap_err();
        let names = error
            .iter()
            .map(|phantom| phantom.name.as_str())
            .collect::<Vec<_>>();
        assert_eq!(names, vec!["a_agent", "b_agent"]);
    }
}
