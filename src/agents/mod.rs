pub mod capability;
/// Multi-agent orchestration system.
///
/// **Capability Agents** (unique skills):
/// - `memory`: Persistent learnings across sessions
/// - `time`: Time/date operations and context
/// - `search`: Web search via Gemini (capability-gated)
///
/// The former workflow agents (`file_loop`, `sequential`, `quality`) and the
/// `orchestrator` that sequenced them were removed in v2: every stage was a
/// placeholder that fabricated results and reported success. Planning is done
/// by the bounded `plan_work` planner tool, and multi-step execution by
/// `crate::workflow`. Requirement 6.3.
pub mod memory;
pub mod search;
pub mod time;
pub mod tools;
