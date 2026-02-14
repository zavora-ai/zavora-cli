# Changelog

All notable changes to this project are documented in this file.

The format is based on Keep a Changelog and this project follows Semantic Versioning.

## [Unreleased]

### Added

- Initial ADK-Rust CLI scaffold with provider-aware runtime.
- Workflow modes: `single`, `sequential`, `parallel`, and `loop`.
- Release-planning command for release-sliced execution plans.
- CI workflow and tag-based release workflow.
- Agile release cycle documentation and release quality gates.
- Selectable session backend with SQLite persistence support.
- Deterministic workflow tests using ADK `MockLlm`.
- `migrate` command for SQLite session schema setup.
- `sessions list/show` commands for session inspection.
