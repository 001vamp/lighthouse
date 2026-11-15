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

## Daemon socket, version 1

This is separate from the notification protocol above. `toastyd` listens at
`$XDG_RUNTIME_DIR/toasty/daemon.sock`. Its directory is 0700, the socket is 0600,
and every connection must have the daemon owner's user ID. A held `flock` prevents
another daemon from replacing a live socket. `--runtime-dir` and `--data-dir`
allow isolated instances.

Each frame is one JSON object followed by a newline. Client frames are limited
to 1 MiB. A client starts with a hello within ten seconds:

```json
{"protocol":1,"client":"my-client"}
```

The daemon answers `{"type":"hello","protocol":1}`. A mismatched version is
rejected. Commands have a numeric client-chosen `id` and a `method`:

```json
{"id":1,"method":"run","agent":"claude","cwd":"/absolute/project","prompt":"Explain this project","config":[]}
{"id":2,"method":"list"}
```

A command response contains `result.Ok` or `result.Err`. For example:

```json
{"type":"response","id":1,"result":{"Ok":{"type":"thread","data":{"id":"..."}}}}
{"type":"response","id":2,"result":{"Err":{"code":"conflict","message":"a turn is running; use steer to add a message or cancel it first"}}}
```

The thread above is abbreviated. The complete schema is in
`crates/toasty-proto/src/lib.rs`. Error codes are `invalid_input`, `not_found`,
`conflict`, `agent`, `storage` and `unavailable`. Malformed frames without a usable
command ID receive a top-level `error`, or the connection closes if framing fails.

| Method | Additional fields | Result |
|---|---|---|
| `run` | `agent` (`claude` or `codex`), absolute `cwd`, nonempty `prompt`, optional `config` pairs | Saved thread; turn continues asynchronously |
| `list` | None | Threads ordered by last change |
| `messages` | `thread_id`, optional nonnegative `after` (default 0), `limit` from 1 to 500 (default 100) | A bounded page of durable text chunks |
| `requests` | Optional `thread_id` | Open approvals and questions |
| `reply` | `thread_id`, nonempty `text` | Starts a turn; rejects while another turn or follow-up runs |
| `steer` | `thread_id`, nonempty `text` | Adds text to the running turn; reports unsupported or ended turns |
| `cancel` | `thread_id` | Asks the agent to stop; final thread state arrives as an event |
| `answer` | `thread_id`, `request_id`, typed `answer` | First valid answer wins |
| `resume` | `thread_id` | Reopens a saved session if needed; returns its thread |
| `close` | `thread_id` | Stops the process group and closes the thread |
| `subscribe` | None | Switches this connection to an event stream |

A `messages` reply contains `{"messages":[...],"next_after":42}` inside its
`data` object. Fetch the next page with `after:42`. A null `next_after` means the
reader caught up at query time. Pages aim for 256 KiB of serialized chunks. A
single larger chunk is returned alone so the cursor always advances. Chunk
sequences remain unchanged across pages. This replaces the earlier unbounded
array reply in the unreleased version 1 protocol.

Example permission answer:

```json
{"id":3,"method":"answer","thread_id":"...","request_id":"...","answer":{"type":"permission","answer":{"type":"selected","option_id":"allow_once"}}}
```

Option IDs come from the request. Never infer an option ID from its label. A
permission answer can instead be `{"type":"cancelled"}`. A question answer is
`{"type":"question","answer":{"action":"accept","values":{"fruit":"ORANGE"}}}`.
Questions also accept actions `decline` and `cancel`, without `values`.

Question forms contain either named fields or a URL. Field kinds are `text`,
`number`, `integer`, `boolean`, `choice` and `multi_choice`. Values keep their JSON
types. Multiple selections use an array of distinct supplied choice strings.
Required fields must be present. Unknown fields, wrong types, duplicate selections
and choices outside the supplied list are rejected without closing the request.
Accepting a URL form uses an empty values object. URLs are displayed to the user;
the daemon does not open them. Dismissing a view sends no answer.

### Subscription and persistence

After a `subscribed` response, the connection receives
`{"type":"event","event":{"type":"thread_changed","data":...}}` and other typed
events: `message`, `request_changed`, `tool`, `usage` and `error`. Send commands on
another connection. The stream has a bounded buffer. If it falls behind, the
server sends `resync_required` and closes it. Slow writes time out after ten seconds.

Subscribe before fetching `list`, `requests` and `messages` on another connection.
Buffer events during those reads, then refresh thread/request snapshots when
buffered changes overlap them. Message sequences and request IDs support deduplication.
On reconnect, fetch fresh snapshots. Events are not replayed from an in-memory log.

Thread states, agent text chunks and requests are committed to SQLite before
publishing their corresponding events. A session ID is committed before the first
prompt. Saved mode/model settings are reapplied on resume. User prompts and
follow-ups are saved before sending them to the agent. A failed injection leaves
a system message explaining that the follow-up was not injected.

On startup, running/waiting threads become interrupted, dead-process requests are
withdrawn, and interrupted sessions reopen independently. Saved scrollback and
partial replies remain available. Reopening restores context; it does not
re-execute an unfinished prompt. New replies explicitly start the next turn.
Clean shutdown preserves idle and failed states. Only a session with an active
turn becomes interrupted on shutdown. Closed threads cannot reopen. Idle and
failed threads reopen when explicitly used.
An agent error affects its own thread; other sessions and clients continue.

Each adapter runs under a supervisor in its own process group. If the daemon is
killed, the kernel signals the supervisor, which kills the entire group. A private
process registry also permits cleanup after an abnormal shutdown. Under systemd,
a service control group can provide an additional boundary.

### Backend flow control and watched status

Each adapter connection has bounded queues for incoming and outgoing frames, each
with 64 slots. The session event queue has 256 slots. Incoming adapter frames are
limited to 1 MiB. Incoming queues apply backpressure. An outgoing full queue
returns an explicit busy error, and cancelled calls release their pending entries.
Client commands and agent events share a fair session loop. A turn's completion
or transport failure waits until preceding events have been consumed. Persisted
text therefore precedes its final state without giving streaming events permanent
priority over cancel or approval answers. Resume discards replayed history before
it enters the live event queue. SQLite owns the cached history.

The existing watched-session dock still reads hooks, independently of toastyd.
Explicit usage-limit hook reports produce Paused cards and paused notifications.
A Codex interrupt preserves its card as Paused. New prompt and tool hooks refresh
the last update time. After ten minutes without an update, a Working or Waiting
app card shows Unknown with "No recent status". This is a freshness indicator,
not proof that work stopped. A later hook restores the reported status. Paused
cards remain until dismissed or a new lifecycle event changes them. Done, Paused,
Failed and Unknown app cards have a dismiss button. Opening a Done card also
dismisses it. Dismissal removes only the unchanged dock report, under the same
registry lock used by hooks. Each new hook report has a unique revision, and a
dismissal must match the complete displayed report. Pointer presses retain that
snapshot, so an update during a click cannot dismiss the newer report. Focusing
an app does not acknowledge its other threads. Done app cards have no age-based
hiding rule. It neither stops the agent nor answers a request.
A new turn can create a new card. The dock checks freshness
every thirty seconds, including when the registry file does not change.

Agent text chunks may combine consecutive adapter updates for up to 25 ms or 16 KiB.
Request/tool events, client commands and turn completion force buffered text to commit
before proceeding. Chunk boundaries are storage boundaries, not token or message boundaries.
Unpublished buffered text can be lost on a crash; published chunks remain durable.

## Notification event metadata

`toasty-notify` accepts optional `--session`, `--event`, `--revision`, and `--reason`
fields. They become `x-toasty-session`, `x-toasty-event-revision`,
`x-toasty-report-revision`, `x-toasty-event-time`, and `x-toasty-reason` hints.
The report revision is read from the local session registry when available.
Event time is milliseconds since the epoch. Title refreshes do not write reports.

An event replacement slot is scoped by agent, exact session, and event key.
Completion producers use the `completion` slot. Approval producers use an exact
request ID when the frontend supplies one. Requests without an ID are not merged.
The helper serializes each slot under a file lock and scopes saved notification
IDs to the notification server's D-Bus owner. A repeated nonempty event revision
is ignored. New revisions replace the slot and reset the toast's interaction state.
Neither visible text nor a folder name identifies an event.

`--incomplete-code` sets `x-toasty-code-complete=false`. Permission producers send
full JSON tool input up to 16 KiB. Beyond that limit, they send an explicit review
notice and omit all approval actions. The shell hides approval actions for a
visually truncated request until its full supplied content is expanded. Dynamic
text uses plain text, including away rows, code, context, and captions.

The shell retires linked completion and waiting events after an authoritative new
registry revision supersedes them. It checks request receiver markers periodically
and again before a button action. Without matching report metadata it does not
infer lifecycle state. Closing, opening, timing out, or focusing a toast supplies
no permission decision. Desktop app focus does not prove exact-session focus.
