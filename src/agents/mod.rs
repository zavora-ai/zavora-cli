/// Multi-agent orchestration system.
///
/// This module implements a capability + workflow agent architecture:
///
/// **Capability Agents** (unique skills):
/// - `memory`: Persistent learnings across sessions
/// - `time`: Time/date operations and context
/// - `search`: Web search via Gemini (capability-gated)
///
/// **Workflow Agents** (execution patterns):
/// - `file_loop`: Comprehensive file discovery
/// - `sequential`: Plan and execute with tracking
/// - `quality`: Verify work against criteria

pub mod file_loop;
pub mod memory;
pub mod orchestrator;
pub mod quality;
pub mod search;
pub mod sequential;
pub mod time;
pub mod tools;
