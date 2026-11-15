# Improve Toasty's notification toasts

## Recommended direction

Make each toast a compact event card that answers: which agent and context, what changed, and what can I do? Keep Toasty's pale surface and mascot identity, but put a small mascot inside one stable layout. Lead with a factual status title. Put assistant prose behind an explicit detail control. A turn ending means "Turn finished", not that the user's whole task succeeded.

Scope is notification toasts and their producer payloads only. Leave sidebar titles, dock behavior, and metadata helpers to the parent task. This document is a plan, not an implementation. No tests were added or run, and no project files were changed.

## Evidence and limits

Inspected the supplied screenshot at `the supplied screenshot (kept outside Git)`. It shows Codex, `mew · Now`, literal `**` around text, a prose excerpt cut off mid-sentence, and a mascot above the card. The card does not visibly say that a turn finished or offer an explicit open action.

Read the current working copies under `toasty/`, including `PROTOCOL.md`, `shell/files/notifications/AgentToast.qml`, `shell/pond.patch`, and the Codex and Claude notification, permission, and status hooks. No `AGENTS.md` was found in the repository or checked ancestor directories. Working copies already contain uncommitted changes; this plan describes the inspected state and must be reconciled with the parent's final changes before implementation.

Concrete findings:

- `AgentToast.qml` uses `State.message(notification)` for ordinary messages, but directly uses `summary` for code-bearing prompts. Its normal message text is explicitly `Text.PlainText`, 13 px with 20 px line height, limited to four lines. The literal Markdown is consistent with this rendering. The base `State.message` implementation is not included in the patch, so verify its exact selection behavior in the installed shell before changing it.
- `bin/toasty-codex-notify` handles `agent-turn-complete`, flattens whitespace in `last-assistant-message`, and cuts it to 220 characters before sending `Done in <project>` and the excerpt. Claude's `last_assistant_text` also flattens and cuts transcript text to 220 characters. QML cannot expand either excerpt into a full answer.
- The wire already carries `kind`, `summary`, `body`, `context`, code, actions, reply capability, window ID, and open URL. It does not carry a guaranteed task-result summary, exact toast session key, or turn revision. Context is generally a folder basename, not a proven task title.
- `done`, `needs`, `paused`, `failed`, and `info` exist. The three attention kinds are sticky and critical. Codex status maps interruption to paused in the session registry, but `toasty-codex-notify` emits only turn-complete notifications. A registry status is not evidence that an interruption toast exists.
- Layout currently switches between 384 px and 440 px cards and four mascot arrangements based on content length, stickiness, and reply capability. This creates changing text positions and exterior mascot clearance.
- `AgentToast.qml` already preserves draft and delivery state across updates with the same notification ID, pauses its eight-second countdown for hover or reply activity, honors reduced motion, and checks sender markers. Preserve these behaviors.
- `shell/pond.patch` contains shell integration for urgent preemption, queueing, away summaries, restored waiting notifications, activation, and action stamps. `toasty-notify` currently passes replacement ID `0` to `Notify`, so new producer calls are not automatically replacements. Claude separately suppresses idle prompts after Stop and duplicate permission notifications.

The screenshot alone does not prove a successful task, a failure, an approval request, or which integration produced it. The examples below specify proposed copy, not newly inferred summaries of the user's work.

## 1. Fix the information hierarchy and copy first

Use the same structure for every kind:

1. A 28–32 px mascot beside the agent name, context, age, and a separate dismiss control.
2. A prominent semantic title, such as "Turn finished" or "Approval needed".
3. At most two lines of supporting information, followed by explicit controls.

Keep context separate from status. Use existing context immediately. Use a future task label only when the parent provides a reliable field; do not derive a title from arbitrary assistant prose or reach into sidebar helpers. If context is missing, omit it. Preserve an accessible full label when the visible context is elided.

| Event evidence | Proposed title and supporting information | Action |
|---|---|---|
| Explicit `done` | "Turn finished". Optional literal producer excerpt, identified as "Reply preview" in details. | "Open session" when a target exists. |
| `needs` with a live permission receiver and supplied actions | "Approval needed", followed by the supplied question and exact command or file detail. | Supplied Allow and Deny options, plus "Review in session". |
| `needs` without permission actions | "Input needed", with the supplied question. Do not imply approval. | Reply only when supported and live; otherwise open the session. |
| `failed` | "Couldn't finish", with a supplied reason if available. | "Open session". A suggested reply must say what it sends. |
| `paused` with explicit interruption reason | "Interrupted". | "Open session". Do not resume work automatically. |
| `paused` with explicit usage-limit reason | "Paused", then "Usage limit reached". | "Open session". Do not invent a reset time. |
| `paused` without a reason | "Paused". | "Open session". |
| `info` | Use the supplied title, with a neutral presentation. | Only supported actions. |

Keep completion copy deterministic. Prefer a generic truthful title to an invented claim such as "Changes verified" or "Sidebar fixed". A future producer-supplied outcome may be displayed as such, but adding an LLM summarizer is outside this plan.

Acceptance: a glance identifies status without interpreting prose, mascot position, or color. The screenshot's content would appear beneath "Turn finished", with optional reply details, rather than become the toast's main message.

## 2. Make formatting safe and approvals complete

Keep all untrusted text plain by default. For assistant-prose previews only, convert Markdown to display text through a bounded, non-executing conversion: retain visible link labels, paragraph boundaries, and list text; remove recognized emphasis and heading delimiters. Preserve unmatched punctuation, paths, identifiers, and code content. Never strip every asterisk or underscore with a global replacement. Unknown syntax remains literal in details.

Do not enable arbitrary `Text.RichText` or auto-detected rich text to hide the markers. Explicitly set plain text on code and caption fields too. Away-summary rows currently assemble unescaped dynamic strings into `Text.StyledText`; replace that composition with separate plain-text items or escape every dynamic value. Treat strings such as `<img ...>`, `<b>`, and ampersands as content.

Permission details must retain their exact meaning. Both permission producers shorten descriptions through `one_line`; moving that already-shortened string into an expandable box cannot recover the command. Preserve full approval detail at the producer, with an explicit completeness flag or bounded full-detail field if needed. If complete review is unavailable or exceeds a payload bound, show "Full command available in session" and require review there before offering toast approval. Do not present an abbreviated command as sufficient evidence for approval.

Acceptance: `**bold**` becomes readable preview text, code containing `**` or underscores remains exact, and markup-like input never renders external content. Every abbreviated approval clearly exposes its limitation and cannot be approved based solely on hidden material.

## 3. Stabilize sizing, reading, and actions

Start with a 420 px card, capped to the output's usable width with at least 16 px outer margins. Use 16 px internal padding, 8 px spacing, a 15 px semantic title, and 14 px body text with about 20 px line height. These are starting design values to verify visually, not measured accessibility results. Keep close and text columns separate. Remove exterior mascot overlap and content-length-driven mascot movement. Use one quiet shadow and existing pale colors initially.

Use status text and a small status symbol, with a restrained colored accent for attention. Avoid shaking failure cards or continuously breathing waiting mascots. Keep any entry motion short and disable it under reduced motion.

Provide "Show details" only when additional content exists; it expands locally without opening the agent. Use "Hide details" to collapse. Keep the heading and action row visible, cap expanded content to roughly half the usable output height, and scroll the detail region. Long code and unbroken paths must remain reachable without widening the card off-screen. If only a 220-character excerpt is retained, label it "Reply preview" and offer "Open session"; never call it the full answer. Full reply expansion requires changing the producers to retain bounded source text, not merely increasing `maximumLineCount`.

Make session navigation explicit and keyboard reachable. Body click may remain a convenience, but must never answer a request. Show "Open app" if only app-level targeting is available, and no navigation action if no target exists. Dismiss closes the notification without implying Deny, success, or cancellation. Keep Allow and Deny comparable in weight; keep broader permissions secondary and only when supplied. Preserve marker-backed sending, sent, delivery failure, and handled-in-session feedback.

Acceptance: no clipped controls or mascot collision at normal and increased text scale. Completion remains compact. Approval detail and all action labels are readable. Expanding, dismissing, and opening are visibly distinct operations.

## 4. Make lifetime follow event identity

Keep eight seconds as the initial routine-toast duration, beginning when actually shown. Pause for hover, any keyboard focus within the card, expansion, scrolling, and a reply draft. The current `held` checks only field focus and text, so extend it to other controls. Sticky attention events stay until resolved, dismissed, or explicitly withdrawn. Never steal focus when a toast arrives.

Add optional producer event identity before implementing semantic deduplication: session key, turn or request ID, revision, and event time. These are proposed fields, not existing toast data. Deduplicate the same event revision; update the same request in place. Replace obsolete completion events from the same session when a new turn is explicitly observed. Keep different sessions and different approval requests separate, even if their visible text and folder names match. Do not deduplicate by text alone.

Use the latest known lifecycle event to withdraw superseded waiting or completed notices. Without a reliable lifecycle link, retain the event with its original age and avoid claiming it is still current. Preserve draft and delivery state only for the same request, never across a new approval. Re-check receiver liveness before action dispatch, not just at initial display. Keep late replies and stamps bound to the originating notification.

Preserve urgent preemption, away aggregation, and reload behavior from `shell/pond.patch`, but filter resolved events before showing queued or away entries. Do not reset the event timestamp on replacement. Restore only genuinely live attention requests with their original age. A dead receiver becomes "Request no longer available" with navigation, never working approval buttons.

Preserve focus suppression, but distinguish exact-session focus from app focus. `already_focused()` uses app ID for desktop notifications, so one focused Codex window can suppress another thread's event. Only suppress or withdraw for exact target focus when that can be proved. With app-only evidence, do not claim the request was handled. Focus, dismissal, and timeout must never become a permission decision.

Acceptance: duplicate callbacks produce one event; a newer event retires an obsolete one; unrelated sessions survive. Queue return does not revive a resolved approval. A focused sibling thread does not swallow a needed approval. These require identity plumbing before they can be guaranteed.

## 5. Verify the real toast directly

Do this after implementation in a disposable local session. Add or run no tests. Use the actual notification path and installed QML, not an HTML imitation. First confirm which shell copy and `State.message` implementation are loaded, since repository files and installed files can differ. Coordinate shared patch changes with the parent and avoid sidebar or metadata edits.

Capture actual desktop screenshots for the following cases and record observed behavior beside each acceptance criterion:

- A completion using the supplied screenshot's prose, plus empty text, Markdown, markup-like text, long paths, and multiline content. Compare against the original screenshot for status visibility, spacing, and absence of exterior overlap.
- Each event kind in the table, including interruption and usage-limit inputs only where a producer explicitly supports them. Record missing producer coverage instead of manufacturing a state from prose.
- Short and long approvals with full details, including a command exceeding the preview limit. Use a disposable permission receiver, never a live consequential operation. Confirm dismiss and open send no approval decision, and that only the selected supplied option is returned.
- Expand, collapse, scroll, keyboard traversal, Enter and Space activation, and dismissal. Every control needs a visible focus indicator and an accessible name. Expose agent, status, context, and details to assistive technology; do not announce every age tick or decorative mascot. Verify contrast against the rendered colors, with 4.5:1 for ordinary text and 3:1 for control boundaries and focus indicators.
- Normal and increased text scale, narrow usable output, light and dark desktop backgrounds, and reduced motion. No action row should leave the visible area. The close target should be at least 32 px and spaced from adjacent controls.
- Observe expiration with a clock, then repeat while hovering, tab-focused, expanded, and typing. Verify sticky events remain and routine events resume timing after interaction ends.
- Send duplicate and replacement events, two separate sessions sharing a folder name, and a burst containing completion plus approval. Resolve a queued request, kill its receiver, and reload the shell with a disposable pending request. Confirm ages, ordering, disabled stale actions, and absence of repeat announcements.
- Open the exact desktop thread and kitty target where supported. Focus a different thread in the same app. Verify focus suppression and navigation destination directly. Simulate delivery failure and verify the toast never claims "Sent" without the delivered marker.

Completion requires screenshots and observed interaction notes from the actual shell. This planning pass inspected source and the supplied screenshot only; it did not verify a redesigned toast live.

## Delivery order

Implement steps 1 and 2 first so event meaning and safe text handling are settled. Apply the stable layout and detail controls next. Add lifecycle identity and replacement only after producer coverage is explicit. Finish with the direct verification above. Keep this as one coherent toast design; do not expand the work into a notification center, sidebar redesign, or generated task-summary service.
