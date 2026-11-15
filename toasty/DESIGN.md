# Toasty design: from toasts to an agent layer

Status: plan. The ACP spike passed on 2026-11-11 (see [Spike results](#spike-results)); the daemon is not built yet. Written 2026-11-11.

Today Toasty watches agents and shows a toast when one needs you ([README](README.md)). This
document is the plan for the next step: start, follow and answer agents from the desktop itself,
without opening an app.

## The idea

Agents made waiting the main activity. The agent apps were designed for active chatting, so they
make you go to them and they take the whole screen. Toasty brings the agent to you:

1. **Summon.** Press a key and a small box appears. Pick an agent and a project, type a prompt,
   press Enter. The box disappears.
2. **Work.** The agent runs in the background.
3. **Notify.** A card appears when it finishes, fails, or needs an answer.
4. **Respond.** Click or press a key to read the reply, answer or approve in place, or open the
   thread panel to scroll back.
5. **Go deep.** One button opens the same session in a terminal or editor.

Toasty is a control surface, not a harness. The agent's own runtime (Claude Code, Codex) does the
work underneath.

**The gap.** Agent overlays on Linux today either watch and approve, or summon a single agent
from an app window. None runs the whole loop (summon, prompt, card, reply) across several
agents, natively on layer-shell. That loop is what Toasty is for, so the summon box is built
before any further card work.

## Who it is for

- **First:** a developer on a Wayland tiling compositor (niri, Hyprland, Sway) running two to
  five agents at once.
- **Second:** any Linux developer, including GNOME and KDE, with a smaller version that still
  works.
- **Not yet:** non-developers. Agents that edit code need a project folder and approvals.

## Depth levels

| Level | What it is | Who provides it |
|---|---|---|
| 0. Ambient | Cards and the dock: working, done, needs you | Toasty |
| 1. Peek | The last reply, Allow/Deny, a quick reply | Toasty |
| 2. Thread | The panel: scrollback, thread switching, a composer | Toasty |
| 3. Workbench | Diffs, terminals, file browsing | The terminal, an editor, or a full agent app |

Toasty does not rebuild level 3. It makes levels 0 to 2 fast enough that most interactions never
need it, and hands off with one button when they do.

## Decisions

| Decision | Choice | Why |
|---|---|---|
| Shape | A daemon, `toastyd`, plus thin clients | The daemon owns agent sessions. The overlay, the command line and notifications are views of it. |
| Agent protocol | ACP (Agent Client Protocol) first, behind an internal backend interface | One open standard covers Claude Code, Codex, Gemini, OpenCode and more. The interface lets a native backend replace an adapter that falls short. |
| First agent | Claude Code, then Codex | Claude has the most mature ACP adapter. Codex second proves nothing was built for Claude only. |
| Adapter versions | Pinned: `@agentclientprotocol/claude-agent-acp` 0.106.0 and `@agentclientprotocol/codex-acp` 2.6.0 | The adapters ship almost daily. These two versions passed the spike. `@zed-industries/codex-acp` is deprecated in favor of the second. |
| Codex backend | The ACP adapter, not Codex's app-server | The adapter already drives `codex app-server` underneath and passed every check. The app-server stays the fallback if a later adapter version loses a feature. |
| Daemon language | Rust | The daemon is one binary that runs on any distribution. ACP's reference library is in Rust. |
| Approvals | Owned by the daemon | The card, the panel and the command line can all answer one request. The first answer wins. |
| Overlay | A standalone Quickshell app | Keeps the existing card work and runs on any layer-shell compositor. Pond is no longer required. |
| Fallback | Plain desktop notifications with buttons | The [wire protocol](PROTOCOL.md) already does this, so every desktop gets Allow/Deny and reply. |
| Keybinds | The `toasty` command | Every compositor can bind a key to a command, so nothing is tied to niri. |
| Storage | A small SQLite index of threads | The agents keep the real history. Toasty keeps the list and a cache for scrollback. |

### What ACP does not remove

- **Node is still needed.** Claude's ACP adapter is an npm package, and Claude Code's own
  installer no longer brings Node with it. Packaging must check for Node or supply it.
- **Adapters may lag the agent.** T3 Code drives Claude through the Agent SDK and Codex through
  its own app-server, not through ACP. Codex's ACP support is a separate adapter. If it cannot
  carry approvals, questions and steering, Codex gets a native app-server backend behind the same
  interface.
- **Requests are more than two buttons.** Some agents ask questions as small forms, so Peek
  renders a request as a list of typed fields from the start.

### Why not build on T3 Code's server

T3 Code runs a local server that already drives every agent, and Toasty could be another client
of it. That was rejected: its RPC contract is internal and changes fast, and it would make T3 a
required install. ACP gives the same reach through a published standard.

## Engineering principles

Toasty is built to be fast and to scale, with no shortcuts.

- **Real protocols, never scraping.** Owned sessions get state, replies and approvals from ACP.
  Reading window titles and typing into a terminal stay only on the watched path, and no new
  feature may depend on them.
- **The daemon never blocks.** It is async throughout. One slow or crashed agent cannot stall
  another session, the socket, or a card.
- **Events are streamed, not polled.** Clients subscribe and receive changes. Nothing re-reads
  whole threads or files on a timer.
- **State survives a crash.** A thread is written to SQLite before it is shown, so a daemon
  restart loses nothing and resumes live sessions.
- **The socket protocol is versioned and typed** from the first commit, so the overlay, the
  command line and other shells can update independently.
- **No tests; correct by construction.** The owner's rule (2026-11-11): Toasty has no test
  suite. Code must do exactly what it says. Types make invalid states impossible, input is
  checked where it enters, every error has a clear type, and nothing panics. CI only builds,
  lints and checks formatting. Because the adapters are outside our control, a new adapter
  version is tried by hand before its pin moves. The recorded spike transcripts stay as
  reference for how each agent behaves.
- **Budgets are measured.** Summon box visible within 50 ms of the keypress, and a card on
  screen within 100 ms of the agent's event. A change that breaks a budget does not merge.

## Architecture

```
 hooks (agents you start yourself) ──┐
                                     ▼
 toasty CLI ──┐                 ┌─────────┐ ──ACP──▶ Claude Code, Codex, …
 overlay   ───┼──unix socket──▶ │ toastyd │
 shells    ───┘                 └─────────┘ ──D-Bus─▶ notifications (PROTOCOL.md)
                                     │
                                  SQLite
```

- **`toastyd`** starts and resumes agent sessions over ACP, records threads, and turns agent
  events into toasts. It listens on a Unix socket at `$XDG_RUNTIME_DIR/toasty/` and speaks
  line-delimited JSON. It runs as a systemd user service where systemd exists, and is otherwise
  started on demand by the first `toasty` command.
- **`toasty` (command line)** is a client of that socket: `toasty run "…"`, `toasty ls`,
  `toasty reply`, `toasty panel toggle`. Keybinds and scripts use it.
- **The overlay** is a Quickshell app that reads the same socket. It draws the summon box, the
  cards and the thread panel.
- **Notifications** stay as they are. `toastyd` becomes one more sender of the existing protocol,
  so a Toasty-aware shell such as Pond keeps drawing its own cards.

### Two kinds of session

- **Owned:** started from Toasty. The daemon drives them, so replies, approvals, history and state
  come straight from the agent.
- **Watched:** started by you in a terminal or a desktop app. The existing hooks report them.
  Approvals already work through the permission hooks. Only replies go through kitty.

Both appear in one list.

### Keeping hooks out of owned sessions

Both agents load the owner's normal settings, so Toasty's own hooks also run inside sessions
Toasty starts. In the spike, Codex's permission hook caught an approval before it reached ACP,
and Claude's hooks sent their usual events. The daemon starts every adapter with
`TOASTY_OWNED=1`, and every Toasty hook exits at once when it sees it. Other hooks the owner has
still run.

### Approvals

- **Owned sessions wait.** A request stays open until someone answers it. The daemon never
  denies or times out on its own.
- **Dismissing a card is not an answer.** The request stays open in the panel and the command
  line, and the agent keeps waiting.
- **Watched sessions expire.** The permission hooks wait 570 seconds, under the agent's
  600-second hook timeout. After that the agent's own terminal prompt takes over.

### When the daemon stops

Agents stop with the daemon. Each thread is marked interrupted and resumes when the daemon
returns. To make that possible, the daemon writes the agent's session ID to SQLite when the
session is created, before the first prompt is sent.

Being a child process is not enough. Claude's adapter quits when its input closes, but in the
spike Codex's adapter kept running after its parent was killed. So the daemon starts each adapter
in its own process group with a supervisor watching a dedicated daemon-lifetime socket,
kills the whole group when a session ends,
and on start kills any group left over from a crash. Under systemd, the service's control group
covers this as well.

### The socket

The socket and its folder are owner-only (0600 and 0700), and the daemon checks the caller's
user ID. That keeps other users out. It does not keep out other programs running as you, which
is the same limit the [wire protocol](PROTOCOL.md) states for notifications. Compositor-specific features (raising a window, skipping a toast for a
window you are looking at) stay optional modules per compositor.

### Three tiers of support

1. **Everywhere:** daemon, command line, plain notifications. Any distribution, any desktop.
2. **Layer-shell compositors:** the Quickshell overlay.
3. **Shell integrations:** shells that implement the wire protocol draw cards natively.

## Out of scope for the first version

- Diffs and a built-in terminal
- Agents delegating to other agents, and handoffs between agents
- Phone or remote access
- X11 polish

## Build order

1. **Spike.** Drive one Claude session over ACP from the command line. It passes only if all of
   these work:
   - a prompt and its reply;
   - an approval and an agent question, both arriving as requests;
   - reopening the session with `claude --resume`;
   - killing the daemon mid-run, then resuming the thread.

   Also run Codex's ACP adapter through the same checks, so step 5 is not a guess.
2. **Daemon, command line, notifications.** Useful on every distribution on its own.
3. **Summon box** in Quickshell, reusing today's cards unchanged.
4. **Thread panel** with scrollback, then card changes for Peek.
5. **Codex**, then merge watched sessions into the same list.
6. **Packaging:** AUR first, then an install script.

## Spike results

Run 2026-11-11 with throwaway code in `spike/`. Full evidence and recorded transcripts:
[spike/RESULTS.html](spike/RESULTS.html). ACP is good enough for
both agents, so the plan stands.

| Check | Claude (adapter 0.106.0) | Codex (adapter 2.6.0) |
|---|---|---|
| Prompt and reply | Pass | Pass |
| Approval as a request | Pass: `session/request_permission`, with a diff for edits | Pass: `session/request_permission` for a shell command |
| Question as a request | Pass: `elicitation/create`, a typed form | Pass: `elicitation/create`, both blocking and async forms |
| Steering a running turn | Advertised, not tried | Pass: `_session/steering` injects into the running turn |
| Cancel | Not tried | Pass: `session/cancel` ends the turn as `cancelled` |
| Reopen in a terminal | Pass, headless (`claude -p --resume`) | Pass, headless (`codex exec resume`) |
| Kill mid-run, then resume | Pass: `session/resume` and `session/load` | Pass with `session/load`. The adapter survived its parent's death |

What this changes:

- **The session ID is the agent's own thread ID** for both agents, so handing off to a terminal
  needs no lookup table.
- **Questions are forms.** Both agents send a JSON Schema with choice fields and an optional
  free-text field. Clients must declare `elicitation.form` in `initialize`. Codex also has async
  questions that the agent does not wait for, so Toasty must show those differently from
  blocking requests.
- **Steering uses an extension.** Mid-turn messages go through `_session/steering`, not a
  second `session/prompt`. A second prompt waits for the turn to end, and cancelling the turn
  does not clear it.
- **Resume on restart** with `session/resume` where the agent supports it, since it is cheaper.
  Use `session/load`, which replays history, only for scrollback the cache lacks. A half-written
  Claude reply is kept and marked interrupted.
- **New sessions need explicit settings.** Claude starts in its Auto mode with a model the SDK
  picks. Codex rejected the `untrusted` approval policy, and `on-request` works. The daemon sets
  the mode, model and policy the owner picks.
- **Process trees and hooks** need the two fixes above.

## Not yet verified

- **Resuming in an interactive terminal.** Both agents resumed headless (`claude -p --resume`,
  `codex exec resume`). The interactive commands read the same files but were not opened.
- **Several clients answering one request at once**, and requests waiting for hours. Not tried
  in the spike. The approval broker is built so that only one answer can ever win.
- **Quickshell on Debian and Ubuntu.** It may not be packaged there. Those users would start on
  tier 1.
- **The speed budgets.** 50 ms for the summon box and 100 ms for a card are starting targets,
  not measurements. Confirm or adjust them once there is something to time.
- **Quickshell and the daemon socket.** Reading and writing a Unix socket from Quickshell is
  expected to work but has not been tried.


## Core daemon implementation (Nov 11)

The six-crate Rust workspace now connects the ACP backend, SQLite store and approval
broker in `toastyd`. Each session has one worker that owns its lifecycle and request
callbacks. Prompt and steering futures run separately, so approvals and cancellation
remain serviceable while a turn runs. Independent sessions do not share a lifecycle lock.

The local socket is implemented and documented in PROTOCOL.md. It exposes thread
creation/listing, scrollback, reply, steering, cancel, approval answers, resume, close
and subscriptions. Writes precede state/message/request events. Requests have no timer;
closing a client view leaves them open. Competing answers serialize through the owning
session worker, and the first valid answer is saved before reaching the agent.

The store adds a second migration for saved session settings. Recovery retains IDs,
settings and cached text, withdraws dead-process requests, and reopens interrupted
sessions without rerunning their unfinished commands. A separate process supervisor
handles daemon death and kills each adapter's complete process group. Adapter installs
serialize per agent and retain their exact version pins.

Direct runs used an isolated `/tmp/toasty-core-live` workspace. Codex replied through
the socket, submitted a typed question and a shell approval, accepted an answer, and
wrote the approved file. An invalid question answer left its request open; two concurrent
answers produced one success and one conflict. Steering and cancellation returned the
thread to idle. Killing the daemon left no live processes in its six-process adapter
groups. Reopening retained the same session ID and settings, and Codex recalled
`DAEMON-RECOVER-731`. A second crash run recovered automatically from interrupted to
idle, retaining all 49 cached text chunks without replay duplicates. No test suite was added or run. Formatting, locked workspace builds
and Clippy with warnings denied pass locally.

Claude session creation, storage and streamed error handling also ran through the daemon.
Its five-hour account limit prevented a successful fresh Claude turn during this work.
The earlier spike remains the evidence for its normal question/approval behavior.
Numeric, boolean, multiple-choice and URL forms are implemented; only the single-choice
form was exercised with a live agent during this integration.

The existing desktop dock remains hook-based. One old spike registered a session as
waiting and did not send its completion hook. Its completed transcript confirmed the
stale state, which was corrected in the live registry. Both spike launchers now set
`TOASTY_OWNED=1`, as the daemon already does, so they cannot leave new dock entries.

The CLI command handlers, notifications from the daemon, service activation and the new
desktop overlay are later roadmap work. They are not included in the core milestone.

## Review follow-up, Nov 11

The session loop now uses fair scheduling across commands, events and completed
work. Prompt completion crosses an explicit acknowledgement in the event queue,
so the worker saves preceding text before publishing the final state. Transport
failure uses the same ordering boundary. Incoming and outgoing RPC queues hold
64 frames each, and the event queue holds 256 events. Incoming frames have a
1 MiB limit. Cancelled calls remove their pending entries. An outgoing queue
that cannot accept a frame returns a busy error.

Resume suppresses replay before the live event queue. The store remains the
source of cached history. History queries return bounded pages with an exclusive
continuation cursor. Clean shutdown preserves idle and failed states. A running
turn becomes interrupted and reopens after restart without rerunning its prompt.

Hook-based cards use Unknown when their Working or Waiting report is more than
ten minutes old. A periodic refresh makes the change visible even without another
hook. New lifecycle reports restore the status. Paused cards are retained, and
quota notifications use the same Paused wording and kind. These changes do not
make hooks authoritative or connect the old dock to toastyd.

Direct follow-up runs used `/tmp/toasty-refinement-live`. Codex completed a reply
and saved its full text, returned paginated history, and accepted cancel after live
text arrived. An idle thread remained idle through clean shutdown and restart;
startup opened no adapter groups for idle or failed threads. An active shutdown
marked Codex interrupted; startup recovered the same session, which recalled the
original marker. Reading its full scrollback with a limit of three crossed 38 pages
and ended with a null cursor. Claude again returned
its five-hour quota error. A real paused notification appeared in Pond's IPC state,
and Pond reloaded the changed QML. No tests were added or run. The source builds,
formats and lints without warnings. Claude success, sustained overload and every
question field type have not been exercised in this follow-up.

### Core cleanup (November 11)

Startup failures preserve the protocol error code for all waiting callers. Only an existing
resumed thread is marked failed on setup failure. Each actor owns its request callbacks;
the broker crate supplies validation, without a second settlement map inside the actor.

Agent text is coalesced for 25 ms or 16 KiB. Non-text events, client commands and turn
completion force a flush. Each resulting chunk commits with synchronous=FULL before
publication. This reduces small write transactions while retaining durable published events;
a crash can discard text still in the unpublished buffer. No throughput claim was measured.

The supervisor inherits a dedicated lifetime socket reader and closes it on adapter exec.
The daemon retains the close-on-exec writer; daemon exit produces EOF regardless of the
spawning worker thread's lifetime. EOF, termination signals or adapter exit kill the group.
The unsupervised backend fallback still uses a parent-death signal.

Formatting, locked workspace builds and Clippy passed. No tests or live agent runs were
performed for this cleanup; earlier direct-run evidence above predates these changes.
