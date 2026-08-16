---
name: capability-audit
description: Inspect agent capabilities, skills, subagents, MCP servers, permissions, protocols, and runtime readiness. Use when users ask what the agent can do, which capabilities are installed or connected, why a tool is unavailable, or whether an operation is governed and safe.
---

# Capability audit workflow

1. Read the live capability registry instead of relying on model memory.
2. Distinguish catalog recipes, installed skills, enabled packs, configured servers, connected tools, and authorized actions.
3. Report unavailable dependencies without implying that installation equals connectivity.
4. Include the relevant specialist agent and approval requirement.
5. Use `/capabilities`, `/skills`, `/agents`, `/mcps`, `/inspect`, and `/doctor` data consistently.
