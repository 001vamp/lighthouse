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
| `x-toasty-kind` | string | `info`, `done`, `needs` or `failed`. `needs` and `failed` are sticky: they never time out. |
| `x-toasty-context` | string | A short label shown beside the time, usually the project folder. |
| `x-toasty-code` | string | Text shown in monospace under the title, e.g. the exact command waiting for approval. The body becomes its caption. |
| `x-toasty-suggestions` | string | Newline-separated one-click replies. |
| `x-toasty-window` | string | The compositor window id of the agent's terminal (niri today), so a click can raise it even after the sender has exited. |
| `x-toasty-open` | string | A link to open on click instead, e.g. `codex://threads/<id>`. |
| `urgency` | byte | `2` (critical) for `needs` and `failed`, otherwise `1`. |
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

- **`sessions.json`** holds desktop-app sessions, which have no terminal title to read: `{"<session id>": {"agent", "project", "state", "time", "app", "open"?}}`. `state` is `working`, `waiting`, `done` or `failed`. Writers lock `sessions.json.lock` with `flock` and replace the file in one step.
- **`failures.json`** holds terminal agents whose last turn failed: `{"win:<window id>" | "<kitty pid>": {"error", "time"}}`.

## Security notes

- A notification server can't tell who sent a toast. Anything running as you can show a toast that looks like Toasty. That's why:
  - An Allow or Deny click only ever goes back to the process that sent that toast. A fake toast can't approve a real agent's request.
  - Replies are typed into a terminal only when its foreground program is the agent named in the registry, never into a shell. Every attempt is checked again right before typing.
- What a fake toast *can* do is ask you to type something. Treat a toast asking for a password or secret as fake: agents never ask that way.
- Other programs running as you can see typed replies on the session bus.
