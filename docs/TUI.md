# Interactive TUI

Zavora's full-screen terminal workspace is the primary interactive product surface. It uses the same runner, sessions, tools, guardrails, skills, agents, MCP connections, and permission bridge as classic chat.

## Interaction contract

| Action | Keyboard or command |
|---|---|
| Search every action | `Ctrl+P` |
| Complete a slash command | `Tab` |
| Recall older/newer prompts | `↑` / `↓` |
| Move or delete by word | `Alt+B`, `Alt+F`, `Ctrl+W` |
| Add a prompt line | `Shift+Enter` or `Ctrl+J` |
| Switch BUILD/PLAN | `Shift+Tab` or `/mode` |
| Direct shell mode | `!`, `/shell`, or `!command` |
| Scroll conversation | mouse wheel or `PageUp` / `PageDown` |
| Follow newest output | `Ctrl+End` |
| Cancel active work | `Esc` |
| Clear visible transcript | `Ctrl+L` or `/clear` |
| Exit | `Ctrl+C`, `Ctrl+D` on an empty prompt, or `/exit` |

The header can be clicked to toggle BUILD/PLAN mode. Tool approvals stay inside the alternate screen: `Y` allows once, `T` trusts the tool for the session, and `N` denies the call.

## Runtime commands

The palette exposes runtime inspection (`/status`, `/tools`, `/capabilities`, `/mcp`, `/skills`, `/agents`, `/inspect`, `/doctor`), context and state controls (`/usage`, `/compact`, `/checkpoint`, `/tangent`, `/undo`), model routes (`/models`, `/provider`, `/model`, `/planner-provider`, `/planner`), and work commands (`/todos`, `/delegate`, `/orchestrate`, `/ralph`).

`/sessions` lists persisted conversations, `/sessions switch ID` loads one into the workspace, and `/new [ID]` creates a clean session. `/export [path.md]` writes the visible transcript as Markdown. Shell output is bounded in the display and added to the active ADK session so follow-up agent prompts can refer to it.

## Capability truthfulness

The TUI distinguishes configured MCP servers from tools connected during runtime discovery. It does not label a catalog recipe as usable merely because it exists. `/mcp`, `/doctor`, and `/inspect` expose progressively deeper state without requiring the user to leave the workspace.
