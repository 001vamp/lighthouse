# Toasty wire protocol, version 1

Toasty uses ordinary freedesktop notifications (`org.freedesktop.Notifications`). An agent toast is a normal `Notify` call with a few extras. Any notification server can show it as a plain notification, and a Toasty-aware shell can render it as an agent toast. `toasty-notify` is the reference sender.

## Identity: the app name

```
Toasty|<agent name>|<provider>|<model>|<icon>
```

- Example: `Toasty|Claude Code|Anthropic||`. Empty fields stay empty, so the four `|` are always there.
- The fields can't contain `|` or a newline. Senders refuse them, so one field can't spoof another.
- The agent name is looked up in the agent registry (`agents.json`) by name or alias for its accent colour, mascot and terminal command.
- Shells should also accept the pre-release prefix `PondAgent|`.

## Hints

| Hint | Type | Meaning |
|---|---|---|
| `x-toasty-kind` | string | `info`, `done`, `needs`, `paused` or `failed`. `needs`, `paused` and `failed` are sticky: they never time out. |
| `x-toasty-context` | string | A short label shown beside the time, usually the project folder. |
| `x-toasty-code` | string | Text shown in monospace under the title, e.g. the exact command waiting for approval. The body becomes its caption. |
| `x-toasty-details` | string | Full text behind "More", e.g. the complete request input. Without it, "More" shows the whole message or code. |
| `x-toasty-suggestions` | string | Newline-separated one-click replies. |
| `x-toasty-window` | string | The compositor window id of the agent's terminal (niri today), so a click can raise it even after the sender has exited. |
| `x-toasty-open` | string | A link to open on click instead, e.g. `codex://threads/<id>`. |
| `urgency` | byte | `2` (critical) for `needs`, `paused` and `failed`, otherwise `1`. |
| `resident` | boolean | Always `true`. The shell dismisses the toast itself once it's handled, so it can show what happened ("Sent", "Allowed"). |

Shells should also accept the pre-release names `x-pond-*`. When both are present, `x-toasty-*` wins.

## Actions and replies

- **Buttons:** standard notification actions, as `[id, label, …]`. Ids Toasty uses: `allow`, `deny`, `always`. `default` (clicking the toast body) focuses the agent.
- **Reply field:** the `inline-reply` action (its label is the placeholder). The server answers with `NotificationReplied(id, text)`.
- **Answers:** a sender learns the answer from the `ActionInvoked` and `NotificationReplied` signals, and stops listening on `NotificationClosed`.
- **Withdrawing:** a sender that stops waiting calls `CloseNotification`, for example when the agent's question was answered in the terminal. The shell shows "Handled in the session" for a moment. It never shows "Allowed" or "Denied" for an answer it didn't see.

## Markers

A sender that listens for answers writes files into `$XDG_RUNTIME_DIR/toasty/markers/` (mode 0700):

- **`<id>.alive`** contains the sender's pid, for as long as it listens. A shell shows buttons and the reply field only while this names a running `toasty-notify` process. Otherwise it shows "Sender gone — open the agent to answer."
- **`<id>.delivered`** is written once a reply has really reached the agent, e.g. typed into its terminal. The shell shows "Sent to …" only when this appears, and "Couldn't deliver" when it doesn't.
- **Clean-up:** markers are removed when the sender exits. Markers left behind by a killed sender are cleaned up by the next sender.

## App notifications

An agent's desktop app may send its own notification for the same moment as a Toasty toast. A shell should show one card: if a notification from a registered agent's app (matched by `desktop-entry` or app name) arrives within 3 seconds of a Toasty toast for that agent, in either order, merge them. Keep the app's notification alive so its `default` action can open the session when the card is clicked. One with no partner is shown on its own.

## Agent state files

Hooks report agent state for the dock in `$XDG_RUNTIME_DIR/toasty/`:

- **`sessions.json`** holds desktop-app sessions, which have no terminal title to read: `{"<session id>": {"agent", "project", "state", "time", "app", "open"?, "t3"?}}`. `state` is `working`, `waiting`, `done` or `failed`. Writers lock `sessions.json.lock` with `flock` and replace the file in one step.
- **`t3`** is set for sessions T3 Code started (`T3CODE_THREAD_ID` in the hook's environment): `{"thread", "open", "job"?}`, looked up once with `t3 thread show`. `open` is the thread's `t3code://` link. `job` (`{"id", "title", "open"}`) is the orchestrator thread that spawned the session. The dock folds every session with the same `job.id`, plus the orchestrator's own session, into one job row, and shows a child of its own only while it needs you. `{}` means the lookup failed. Codex sessions in T3 don't get one yet: T3 runs one Codex server for all its threads, so their hooks can't see the thread id.
- **`failures.json`** holds terminal agents whose last turn failed: `{"win:<window id>" | "<kitty pid>": {"error", "time"}}`.

## Security notes

- A notification server can't tell who sent a toast. Anything running as you can show a toast that looks like Toasty. That's why:
  - An Allow or Deny click only ever goes back to the process that sent that toast. A fake toast can't approve a real agent's request.
  - Replies are typed into a terminal only when its foreground program is the agent named in the registry, never into a shell. Every attempt is checked again right before typing.
- What a fake toast *can* do is ask you to type something. Treat a toast asking for a password or secret as fake: agents never ask that way.
- Other programs running as you can see typed replies on the session bus.

## Notification event metadata

`toasty-notify` accepts optional `--session`, `--event`, `--revision`, and `--reason`
fields. They become `x-toasty-session`, `x-toasty-event-revision`,
`x-toasty-report-revision`, `x-toasty-event-time`, and `x-toasty-reason` hints.
The report revision is read from the local session registry when available.
Event time is milliseconds since the epoch. A `needs` toast with reason `approval`
is a permission prompt even when it carries no buttons.

An event replacement slot is scoped by agent, exact session, and event key.
Completion producers use the `completion` slot. Approval producers use an exact
request ID when the frontend supplies one. Requests without an ID are not merged.
The helper serializes each slot under a file lock and scopes saved notification
IDs to the notification server's D-Bus owner. A repeated nonempty event revision
is ignored. New revisions replace the slot and reset the toast's interaction state.
Neither visible text nor a folder name identifies an event.

Permission producers send a one-line summary as `x-toasty-code` and the full JSON
tool input, up to 16 KiB, as `x-toasty-details`. Beyond that limit they send an
explicit review notice, set `x-toasty-code-complete=false` (`--incomplete-code`),
and omit all approval actions. The shell hides approval actions for a visually
truncated summary until details are expanded. Dynamic text uses plain text,
including away rows, code, context, and captions.

The shell retires a linked event once a new registry revision supersedes it:
completion when the session works or waits again, a request when it works, ends,
pauses or fails, and a pause or failure when it works, waits or finishes. A request
may step in front of a sticky paused or failed toast; those return to the head of
the line. The shell checks request receiver markers periodically and again before
a button action. Without matching report metadata it does not infer lifecycle
state. Closing, opening, timing out, or focusing a toast supplies no permission
decision. Desktop app focus does not prove exact-session focus.
