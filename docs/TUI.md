# Interactive TUI

Zavora's full-screen terminal workspace is the primary interactive product surface. It uses the same runner, sessions, tools, guardrails, skills, agents, MCP connections, and permission bridge as classic chat.

## Interaction contract

| Action | Keyboard or command |
|---|---|
| Search every action | `Ctrl+P` |
| List every shortcut | `/keys` |
| Complete a slash command | `Tab` |
| Recall older/newer prompts | `↑` / `↓` |
| Move or delete by word | `Alt+B`, `Alt+F`, `Ctrl+W` |
| Add a prompt line | `Shift+Enter` or `Ctrl+J` |
| Switch BUILD/PLAN | `Shift+Tab` or `/mode` |
| Direct shell mode | `!`, `/shell`, or `!command` |
| Scroll half a screen | `PageUp` / `PageDown`, or the mouse wheel |
| Scroll a whole screen | `Shift+PageUp` / `Shift+PageDown` |
| Scroll one line | `Ctrl+↑` / `Ctrl+↓` |
| Jump between responses | `Alt+↑` / `Alt+↓`, or `Ctrl+O` / `Ctrl+N` |
| Start of conversation | `Ctrl+Home`, `Ctrl+T`, or `Ctrl+Alt+↑` |
| Follow newest output | `Ctrl+End`, `Ctrl+B`, or `Ctrl+Alt+↓` |
| Hand the wheel back to the terminal | `Ctrl+R` or `/mouse` |
| Repaint the screen | `Ctrl+L` |
| Cancel active work | `Esc` |
| Clear visible transcript | `Ctrl+L` twice, or `/clear` |
| Exit | `Ctrl+C`, `Ctrl+D` on an empty prompt, or `/exit` |

Scrolling works while a turn is running, and a view scrolled away from the tail
holds its position: lines arriving below it do not move it. The scroll position
is measured from the start of the conversation rather than from the newest line,
because the newest line moves while a response streams — an offset counted from
there names different content every time output arrives, and earlier output slides
out of reach at the rate it is produced.

While the view is detached, the bottom border reports how far from the tail it is
and names the chord that returns. Returning to the tail by hand re-arms following,
so it is equivalent to jumping there.

`PageUp` and `PageDown` move half a screen rather than a whole one, so the
previous half stays on screen and you keep your place; `Shift` takes the full
screen. `Alt+↑` and `Alt+↓` move by response, which reaches the start of the
previous answer in one keystroke however long it is.

## The mouse wheel is a trade

In the alternate screen a terminal either forwards wheel events to the
application or keeps them for itself; it cannot do both. Posting real wheel
events at Apple Terminal and logging what arrived confirms the choice is binary:

| Requested | Delivered for 3 up + 2 down notches |
|---|---|
| `EnableMouseCapture` | `ScrollUp` ×3, `ScrollDown` ×2 |
| nothing | nothing at all |
| alternate-scroll, `DECSET ?1007h` | nothing at all |

Alternate-scroll mode exists precisely to translate the wheel into arrow keys
without claiming the mouse, and Apple Terminal does not implement it, so there is
no third option there.

The workspace therefore **claims** the wheel. Leaving it to the terminal is not a
neutral choice: screenshots taken before and after eight wheel notches show the
whole drawn frame pushed six rows down the window with blank space above it. The
terminal scrolls its own buffer, displacing a frame the application still believes
it owns, and every diffed redraw after that lands at the wrong row, interleaving
old and new text in the transcript.

The cost is the terminal's own click-drag selection. It is one modifier away —
hold `Fn` in Apple Terminal, `Option` in iTerm2, `Shift` in most others — and the
message shown when the setting changes names the one for the terminal in use.
`Ctrl+R` or `/mouse` hands the mouse back wholesale, and says what that will cost.

`Ctrl+L` repaints every cell, which is the way back from a frame the terminal has
displaced. A second press within two seconds clears the conversation; the first
never does, because reaching for `Ctrl+L` on a corrupted screen should not destroy
the transcript.

`/mouse speed <1-20>` sets the lines moved per notch. It defaults to three, which
matches `vim`. Terminals disagree about how many events one physical notch
produces — some accelerate, others send exactly one — and the application cannot
detect which, so this has to be a setting.

## Handing the terminal back

The workspace puts the terminal into states a shell cannot undo on its own: raw
mode, the alternate screen, bracketed paste, and mouse reporting. All of it is
undone on exit, on a panic, and on a signal.

The signal case matters because it is the one that used to fail. Crossterm's mouse
capture includes any-event tracking (`?1003h`), which reports pointer *motion*
rather than only clicks, so a process killed before it could reset left the shell
echoing `35;1;1M`-shaped escape text on every mouse move — a `SIGTERM`, or simply
closing the window and sending `SIGHUP`, was enough. A signal handler cannot
allocate or take locks, so the reset is written with `write(2)` and the saved
terminal attributes are restored with `tcsetattr(3)`, both async-signal-safe. The
handler then re-raises the signal under its default disposition, so the exit status
still reports the cause.

Order matters: mouse reporting is switched off first, because a terminal still
reporting motion writes into whatever comes next, and the alternate screen is left
last, so the resets land on the screen being discarded rather than on the shell's.
The three paths share one idempotent function, so a signal arriving during a
normal teardown cannot emit the sequences twice.

If a build predating this ever leaves a terminal in that state,
`printf '\033[?1003l\033[?1006l\033[?1049l'; stty sane` recovers it.

## Terminals deliver different keys

Several actions carry more than one chord because terminals disagree about what
they forward. Apple Terminal was measured with a key-event log: it does not send
`Home` or `End` at all, strips `Shift` from `PageUp`/`PageDown`, and does not
send `Ctrl`- or `Alt`-modified arrows — `Alt+↑` arrives as a bare `Esc` followed
by `[` and `A` as separate keys. The `Ctrl`+letter forms are single ASCII control
bytes, so those always arrive.

Run `/keys` to see the shortcuts that reach the workspace on the terminal in use.
It lists only chords that terminal can send, and reports an action as
unavailable rather than naming a key that does nothing. On Apple Terminal, a
whole-screen scroll has no reachable chord; half-screen scrolling, response
jumps, and top/bottom all do. iTerm2, Ghostty, WezTerm, and kitty deliver the
full set.

The footer is generated from the same table the key handler dispatches through,
so it cannot advertise a chord that is not bound or not deliverable. It sheds the
lowest-priority hints rather than overflowing when the terminal is narrow.

The header can be clicked to toggle BUILD/PLAN mode. Tool approvals stay inside the alternate screen: `Y` allows once, `T` trusts the tool for the session, and `N` denies the call.

## Runtime commands

The palette exposes runtime inspection (`/status`, `/tools`, `/capabilities`, `/mcp`, `/skills`, `/agents`, `/inspect`, `/doctor`), context and state controls (`/usage`, `/compact`, `/checkpoint`, `/tangent`, `/undo`), model routes (`/models`, `/provider`, `/model`, `/planner-provider`, `/planner`), and work commands (`/todos`, `/delegate`, `/orchestrate`, `/ralph`).

`/sessions` lists persisted conversations, `/sessions switch ID` loads one into the workspace, and `/new [ID]` creates a clean session. `/export [path.md]` writes the visible transcript as Markdown. Shell output is bounded in the display and added to the active ADK session so follow-up agent prompts can refer to it.

## Capability truthfulness

The TUI distinguishes configured MCP servers from tools connected during runtime discovery. It does not label a catalog recipe as usable merely because it exists. `/mcp`, `/doctor`, and `/inspect` expose progressively deeper state without requiring the user to leave the workspace.
