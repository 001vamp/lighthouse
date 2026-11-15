# Handoff: Toasty v1

Written 2026-11-11 for the next agent session. Read this first, then [DESIGN.md](DESIGN.md) and
[ROADMAP.md](ROADMAP.md).

## Where things stand

- The owner accepted the spike. T1–T6 are Done. Setup and all five core-daemon steps are
  implemented but uncommitted, unmerged and pending owner acceptance.
- `toastyd` now connects the ACP backend, SQLite store and typed approval broker through
  per-thread workers. It serves the private, versioned Unix socket documented in PROTOCOL.md.
- Commands cover run/list, scrollback, requests, reply, steer, cancel, answer, resume, close
  and subscriptions. State is saved before related events. Saved mode/model settings survive
  recovery. Dead-process requests withdraw; unfinished prompts are not automatically rerun.
- A separate supervisor kills each adapter's entire process group when the daemon dies.
  Runtime and database locks prevent duplicate owners. Clients need the same UID; directory
  and socket modes are 0700/0600. Input frames and concurrent client connections are bounded.
- Direct Codex runs exercised prompts, questions, approvals, competing answers, steering,
  cancellation and context recovery after SIGKILL. See DESIGN.md for evidence and limitations.
  Claude created a session and streamed its quota error; its account limit blocked a successful
  fresh turn. Only single-choice questions were exercised live during integration.
- No tests were added or run. `bash tools/check` passes formatting, locked workspace builds
  and Clippy with warnings denied. Remote CI has not run; commit or push only when asked.
- The current desktop dock still uses hooks. The reported stale Waiting entry belonged to an
  old spike, whose completion was confirmed in its transcript. Its live state was corrected.
  Both spike launchers now set TOASTY_OWNED=1, as the daemon already does.
- Follow-up dock fix: removed the nonexistent Codex spike entry ending ece62. The owner's
  Claude session b7a12bf6-3170-4861-ac18-936d1260fb47 is paused for usage limits, not
  working. Sidebar source and installed copy support Paused with a static pause indicator.
  Explicit usage-limit StopFailure/Notification payloads now set paused in the Claude hook.
  Pond reloaded successfully; missing lifecycle events still require authoritative correction.
- Review follow-up implemented Nov 11: fair command/event scheduling, bounded RPC and
  session queues, durable completion acknowledgements, and bounded history pages with
  `next_after`. Resume suppresses replay before it reaches the live queue. Shutdown
  preserves idle/failed states and marks only active turns interrupted.
- Follow-up direct runs confirmed idle/failed preservation, automatic interrupted-session
  recovery with the same ID and remembered context, streaming cancel, and history across
  38 small pages. Claude remains quota-limited. Sustained overload was not exercised.
- Watched sessions now show Unknown after ten minutes without a working/waiting hook
  update. This describes missing information, not a stopped agent. Explicit quota hooks
  and notifications use Paused. Codex interrupt preserves its card as Paused. Pond
  reloaded the source changes; hooks are symlinked to this checkout.
- Core cleanup completed Nov 11: startup replies preserve their original error code;
  failed new-session setup no longer updates a nonexistent thread. Actors own pending
  callbacks directly, using broker validation without retaining broker tombstones.
- Agent text coalesces for 25 ms or 16 KiB, with forced flushes at non-text events,
  commands and turn boundaries. SQLite remains synchronous=FULL; publication follows
  commit. A crash can lose the uncommitted buffer, never an already published chunk.
- Supervised adapters use a dedicated lifetime socket instead of a thread-bound
  parent-death signal. Only the supervisor inherits the reader; the daemon retains the
  writer. EOF kills the adapter group. Unsupervised backend callers retain the old fallback.
- This cleanup passed formatting, locked workspace builds and Clippy with warnings denied.
  No tests or new live agent exercises were run for these changes.
- Dock dismissal now uses `toasty-dismiss`, included in installer helpers and installed as a
  development symlink. App cards have hover-only × controls, also revealed by keyboard focus. Project
  text keeps its width on hover. Opening a Done card also dismisses it.
  The helper locks the registry and checks the displayed timestamp/state before removal,
  so a newer report remains visible. The four Claude cards reported by the owner were
  removed through the live action and confirmed absent from Pond's row model.
- The CLI remains a placeholder. Next engineering work is command-line commands,
  notifications and service activation, followed by the desktop overlay.
- Location: `/home/mew/Documents/Codex/2026-10-30/lo/lighthouse/toasty`, inside the
  `lighthouse` Git repo (remote `001vamp/lighthouse`).

## What Toasty is today

Python hooks for Claude Code and Codex call `bin/toasty-notify`, which sends freedesktop
notifications with `x-toasty-*` hints ([PROTOCOL.md](PROTOCOL.md)). A Quickshell client patched
into the Pond shell (`shell/pond.patch`) draws agent cards, a dock and an away card. Replies are
typed into kitty. Allow and Deny go back through the permission hooks, which wait 570 seconds.
The historical `tests/run` exists, but the owner prohibits adding or running tests.

## What v1 adds

A layer for starting, following and answering agents from the desktop: press a key, type a
prompt, get a card when the agent finishes or needs you. The decisions, with reasons, are in
DESIGN.md. In short:

- **`toastyd`**, a Rust daemon, owns agent sessions. Clients are thin: a `toasty` command line,
  a standalone Quickshell overlay, and plain notifications as the fallback.
- **ACP first**, behind an internal backend interface. Claude Code is the first agent. Codex is
  second, with its own app-server as the fallback if its ACP adapter falls short.
- **Not built on T3 Code's server.** Its RPC is internal and changes fast.
- **Owned sessions** are started by Toasty. **Watched sessions** are started by the owner and
  reported by today's hooks. Both show in one list.
- **Approvals live in the daemon.** The first answer wins, a dismissed card is not a Deny, and
  owned requests wait forever.
- **If the daemon stops,** its agents stop with it. Active turns are marked interrupted and their sessions reopen on
  restart. Idle and failed threads keep their states. The session ID is saved at session creation, before the first prompt.

## How to work with the owner

- **Make the engineering decisions.** The owner is new to engineering and asked not to be given
  menus of technical options. Give one recommendation with a short reason, and say what is
  unverified. Ask only about product taste.
- **No shortcuts.** Fast, scalable, clean. The rules are in DESIGN.md under "Engineering
  principles".
- **Use the `/flow-roadmap` skill** (`~/.claude/skills/flow-roadmap/SKILL.md`). Read ROADMAP.md
  before starting work, update it in the same change that finishes a task, and never mark
  something done that has not merged or been confirmed by the owner.
- **Write plainly.** The owner reads quickly and informally. Explain terms the first time.

## The spike (roadmap node M1)

It passes only if all of these work, for Claude over ACP:

1. A prompt and its reply (T1).
2. An approval and an agent question both arrive as requests (T2).
3. The session reopens with `claude --resume` (T3).
4. Killing the daemon mid-run, then resuming the thread (T4).

Then run Codex's ACP adapter through the same checks (T5), and write the results into DESIGN.md
(T6). Later milestones may change with the results.

Build the spike as throwaway code. The real Rust workspace is task T7, after the spike.

## Facts to check, not trust

- **The Claude ACP adapter** is reported to be the npm package
  `@agentclientprotocol/claude-agent-acp`, around version 0.106, shipping almost daily. Confirm
  the name and pin a version.
- **Resume:** a headless Claude session was seen to resume with `claude -p --resume`. Resuming a
  session started through the adapter is not yet confirmed.
- **Session list, resume and fork** are reported to be draft features in the Claude adapter.
- **Codex's ACP adapter** is a separate project, reported at version 2.6.0. Whether it carries
  approvals, questions and steering is unknown.
- **Quickshell** talking to a Unix socket, and its packaging on Debian and Ubuntu, are untested.
- **The speed budgets** (50 ms summon box, 100 ms card) are starting targets, not measurements.

## The owner's machine

Arch Linux, niri compositor, kitty terminal, fish shell, Pond (Quickshell) as the desktop shell.
Claude Code and Codex are installed with Toasty's hooks. T3 Code is installed, with data in
`~/.t3`.

## Reference: T3 Code

T3 Code's source (`github.com/pingdotgg/t3code`) was read during planning as a model of this
kind of product. Useful files there: `docs/internals/providers.md`,
`docs/orchestration-v2/orchestrator-mcp-server.md`, and
`packages/provider-core/src/server/ProviderAdapter.ts`. Two lessons carried over: put every
agent behind one adapter interface, and test adapters by replaying real recorded sessions.

## Dock follow-up

App focus no longer acknowledges every thread in that app. Done app cards remain
until individually opened or dismissed. The old twelve-hour hiding rule is removed.
Pointer presses capture the card identity and report snapshot. Hooks attach unique
revision IDs, and `toasty-dismiss` compares the complete snapshot under the registry
lock before removing a report. This also covers multiple updates within one second.
The helper takes the session ID and expected report JSON, replacing its earlier
session ID, timestamp and state arguments.

Live Pond use opened one completed card and left its two completed siblings plus
the working card visible. A stale report dismissal left the current report unchanged.
The source and installed sidebar match. No tests were added or run. Physical rapid
clicks and every keyboard interaction have not been exercised.

## Card readability and lifecycle cleanup — Nov 14

Implemented in the hook-based dock; pending owner acceptance, uncommitted.
Terminal activity is inferred from title marks and process discovery, not a task API.
App activity comes from the locked session registry. Codex notify also reports turn completion.

- Task identity and status explanation occupy separate plain-text lines. Raw command-like
  titles, IDs and scratch folder names fall back to “task not available”. Valid prompt text
  supplied by UserPromptSubmit is retained through later app events; older reports cannot
  acquire task identity retroactively. No transcript mining or invented task summaries.
- Claude SessionStart now reports Ready, using the existing hook merge/backup workflow.
  Creation does not mean Working. Codex producer accepts SessionStart, but no new Codex
  startup hook was installed because frontend delivery has not been confirmed.
- Prompt submit means Working; permission means Needs you; tool completion resumes an
  already active/waiting report. Stop means turn finished, not task success. Interrupt means
  Paused. Explicit usage-limit errors mean Paused; other failures ask to open for details.
- Terminal blank-title transitions settle for one second before Ended, then retain the existing
  six-second acknowledgement grace. Initial unread titles alone cannot manufacture completion.
  Terminal Ended remains an inference, and does not trigger the success celebration.
- Working, waiting and ready app reports become Unknown after ten minutes without a hook,
  checked every thirty seconds. This does not imply the process stopped. Finished, paused and
  failed app reports persist until individually handled. Stable row order prevents status jumps.
- Notification body is plain text; command approvals retain their separate code presentation.
  Existing sticky needs/paused/failed and eight-second completion timers remain appropriate.
- Exact report JSON/revision comparisons and per-session open/dismiss are preserved. Ready
  reports are accepted by the dismissal helper. Sibling reports are never acknowledged together.

Validation: Python syntax compilation and qmllint passed. Direct producer flow in a disposable
runtime showed ready → working → waiting → working → paused → working → done with task
retention; an old report dismissal preserved the newer report. No tests were added or run.
Source QML was copied to the existing development Pond install; live IPC returned the updated
cards, including a real Claude prompt supplied during this session. Hooks remain development
symlinks. No CLI/stage 4 work, commits or pushes.

Limits: physical pointer/keyboard races and visual layout have not been manually exercised.
Long/code-heavy prompts use an honest fallback. Hook ordering remains producer arrival order;
without upstream turn sequence numbers delayed async events cannot be perfectly ordered.
Terminal titles cannot prove successful completion or distinguish every shared-kitty session.
Codex frontend startup/prompt payload coverage and Claude interruption delivery remain unverified.

## Narrow card layout follow-up, Nov 14

Cards now reserve two lines for a bounded task excerpt and keep the status pill.
The repeated explanation moved into a native hover/focus tooltip with full source
text. Legacy cards fall back to agent/project, with numbered duplicates; short
acknowledgements use that fallback. Hook titles retain the first supplied prompt
across follow-ups, replacing trivial greetings/acknowledgements when possible.
These are excerpts, not generated semantic summaries. Old reports may still lack
a useful task. Python syntax and QML checks completed; source was deployed to Pond.
No tests, commits or pushes.

## Native conversation labels, Nov 14

`toasty-titles` reads only visible app-session metadata: Codex's local SQLite
name/title by exact ID using a read-only connection; Claude's latest matching
custom-title record from at most 256 KiB of its transcript tail. No model call or
conversation rewrite. Sidebar refreshes labels on reports and every 30 seconds,
without changing registry state, timestamps or revision/dismissal snapshots. The
installer includes the helper; the development symlink and sidebar are deployed.
Live IPC confirmed Find Vibewise overlap and Linux machine app access clarity
replaced raw prompts. Codex's unnamed titles may still be its first prompt; deleted
metadata and Claude titles outside the bounded tail use the existing fallback.
An isolated direct Codex-hook lifecycle went Ready → Working → Waiting → Working
→ Done → removed, retaining the original task across follow-ups. This exercised
hook payloads, not a real new frontend conversation. Physical hover, keyboard and
new frontend lifecycle remain unverified. Python syntax, QML lint and diff checks
completed. No tests, commits or pushes.

## Toast redesign and source publication, Nov 14

User authorized committing and pushing Toasty to `001vamp/lighthouse` on
`docs/toasty-design` for use on other Linux machines. This supersedes the earlier
no-commit instruction for this delivery. The repo is not a GitHub fork. Root README,
hardware and migration changes are unrelated and remain outside this commit.
Private spike transcripts and runtime observations remain local.

Sidebar uses a fixed dark surface and matching styled popup with light text.
The popup uses its own window so the narrow sidebar does not clip its details.
Toasts use a stable inset mascot, factual event heading, explicit open control
when a navigation target exists, and locally expandable, scrollable details.
Reply previews retain up to 16,000 characters; a limit notice identifies truncation.
Paired prose Markdown is converted to display text without interpreting HTML or
altering fenced/inline code. Permission inputs retain full JSON up to 16 KiB.
Requests too large to include cannot be approved in the toast.

Exact event slots support replacement and optional revision deduplication.
Requests without provider IDs are deliberately not deduplicated. Linked registry
updates retire superseded notices. Receivers are checked periodically and before
action dispatch. App-level focus no longer suppresses sibling-thread events.

Direct checks: actual compact and expanded completion captures, Markdown/markup
preview, pause while expanded, duplicate and revised event retaining one ID with
no queued duplicate, and a disposable approval receiver returning action:deny.
A dead receiver displayed Request no longer available with no answer buttons.
Rust formatting, locked build and Clippy passed. QML lint, Python compilation
and portable State/ToastWindow patch round-trips passed. No tests were run.
Increased text scale, narrow-output layout, full assistive technology behavior,
all provider event permutations, delivery failure and away bursts remain unverified.
Native frontend lifecycle coverage still depends on which events each app emits.

Final direct interaction check also captured Space collapsing the focused details
button and confirmed routine expiry after focus and pointer left. Full task hover
uses a separate popup window beside the row to avoid sidebar-window clipping.
