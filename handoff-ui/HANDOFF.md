# UI handoff: Toasty toasts and the agents dock

This note is for the next agent working on Toasty's UI. Jasiel wants to **generate more UI ideas and put each one through a council review**. Read all of it before you change anything. It replaces `handoff-x.md`.

Jasiel is new to engineering. Make the technical calls yourself, and ask only about product taste. Commit or push only when asked.

## Where things stand

- `main` is at `6d2b19e`, the merge of PR #5. Everything below is merged.
  - **PR #4** added slim toasts and grouped T3 Code jobs in the dock.
  - **PR #5** added the Jobs/Agents headings and the UX fixes from the council.
- **Branches:**
  - `docs/toasty-design` still holds the Rust daemon. Don't touch `toasty/crates/**` or `toasty/Cargo.*` from `main` work.
  - `handoff-ui` (this branch) holds this note.
  - Nothing else is open.
- **On this machine**, Pond is installed from `~/lighthouse` on `main` with `bin/toasty install --link`, so the helpers in `~/.local/bin` are symlinks into the checkout.
  - The shell overlay is a copy. Re-run `toasty/bin/toasty install --link` after any QML change to see it live.
  - The installer now handles Pond reloading itself, and never leaves two docks.
- **Tests:** `toasty/tests/run` passes 25 tests. **Write no new tests**; only extend the existing ones.

## What Jasiel likes (the taste guide)

- **Keep the dock base. They "really love" it.** Look at `screenshots/00-original-dock-user-loves.png`:
  - pixel mascots,
  - state-colored pills (Working light, Needs you orange, Done blue with a green check, Paused brown, Failed red),
  - a two-line task,
  - a dimmed mascot when Paused,
  - a hover-only ×,
  - a dark glass card that's **see-through like the rest of the dock**. Never give it a solid fill.
- **Toasts should be slim and never in the way.** Jasiel picked mockup style **B · Quiet** ("non destructive, looks better than this blocky notification").
  - A card only grows when it has something to show: a reply box, a long command, or "More".
- **The dock comes from mockup A · Jobs**, and Jasiel loves the **jobs vs agents distinction** ("sometimes I click a dock thingy thinking it's an agent but it's just a job"). Jobs should always read differently from agents:
  - a dashed outline,
  - the mascot on a stack,
  - Jobs/Agents headings.
- **No Allow/Deny in toasts.** The harness handles approvals, so a "Needs you" toast only offers **Open thread**. The code that still draws approval buttons, when a sender passes them, is the next cleanup (see "Open work").
- Jasiel prefers **seeing it**: HTML pages shown inline, and screenshots of the real desktop. They like mockups with tabs that compare styles side by side, with **transparent backgrounds**.
- **Keep the code lean:** no caches or extra processes without a reason, and fix the actual case.

## How it works today (the parts UI work touches)

**Toast**: `toasty/shell/files/notifications/AgentToast.qml` and `AgentButton.qml`.
- **Layout:**
  - one headline, like "Claude finished · lighthouse", plus the age,
  - a 2-line preview of the agent's last message,
  - text links on the right ("Open" or "Open thread"),
  - "More" or "Less" under the preview,
  - a close badge on the top-left corner, shown only on hover.
- **Sticky toasts** (Needs you, Paused, Failed) have a 2px colored edge.
- **Done toasts:**
  - They close themselves using one clock that drives both the 2px bar and the dismissal. It runs 8 seconds, or 3 seconds while others are queued.
  - The clock pauses while you hover, focus, type or expand, and while you're away.
- **Replying** (the reply box only shows when the session can take one): after you send, the toast echoes "Sent: “…”".
- **"More"** shows the whole message. When a short reason like "Usage limit reached" stands in for it, "More" still shows the real error.

**Dock**: `toasty/shell/files/agents/SidebarCard.qml`. The card is **156px wide**.
- **Rows** are 58px tall, mascot 28px, pill 20px.
- **Where rows come from:**
  - Terminal agents are read from niri window titles.
  - App and T3 sessions come from `$XDG_RUNTIME_DIR/toasty/sessions.json`, written by the hooks.
- **Jobs:**
  - Sessions T3 Code starts have `T3CODE_THREAD_ID`. The Claude hook looks the thread up once with `t3 thread show --json`, after writing the session's state, and stores `{thread, open: t3code://…, job: {id, title, open}}` in the `t3` field.
  - Sessions with the same `job.id`, plus the orchestrator's own session, fold into one job row.
  - **Pill order:** Needs you, then Failed, Paused, Working. It reads "Working 1/3" only while the job is working; the count is agents finished out of agents.
  - **Children:** a child only gets its own indented row while it's waiting, failed or paused.
  - **Clicking a job** opens the orchestrator's thread in T3 (`t3code://threads/<env>/<thread>`).
- **Headings:** "JOBS" and "AGENTS" appear only when both kinds are present, and fold away when the dock collapses.
- **Known gaps:**
  - T3's **Codex** sessions don't group. T3 runs one shared Codex app-server, so Codex hooks can't see the thread id. Don't read T3's private sqlite to work around it.
  - A finished job disappears once T3 closes its children's sessions. Jasiel said that's fine.

Wire format and file contracts: `toasty/PROTOCOL.md`.

## Mockups so far

`mockups/toasty-mockups.html` is one self-contained page with tabs. Open it in a browser, or show it inline with T3's `html_preview` and `html_render`.
- **A + B:** the chosen direction, now built.
- **Today:** the baseline before the slim toasts.
- **A · Jobs:** an orchestrator job row with a segment bar, child rows and a "9 done today" footer.
- **B · Quiet:** slim one-line dock rows and banner toasts.
- **C · Stack:** a deck of toasts with a count and peek chips.
- **D · Waiting time:** an orange wash on rows that need you, a meter that fills over 30 minutes, and a breathing mascot.

To make new styles, add tabs to this page. It already has the real mascots inlined as base64 and the real colors as CSS tokens.

**Ideas not built yet**, all fair game for new mockups:
- A's segment progress bar and "9 done today" footer.
- C's stacked toasts.
- D's waiting-time cues.
- A "While you were away" card sorted by Needs you, then Paused, then Failed.
- One "T3 Code job done" toast per job instead of one per agent.

## The council process (what Jasiel means by "council")

Run it from T3 Code. Skill: `~/.claude/skills/council/SKILL.md`.

1. **Fix what you already know is wrong first**, run `toasty/tests/run` and `qmllint`, and commit. The council reviews the updated work.
2. **Pick the members.** This round Jasiel named them: **you (Claude Opus) + GPT Sol (`gpt-6.1-sol`) + GPT Astra (`gpt-6-astra`), both at medium reasoning, read-only**.
   - Call `orchestrator_capabilities` once, then one `delegate_task` per model with `role: "review"`, `mode: "async"` and a new `clientRequestId` each round.
   - You review as the third member. Don't change models without asking.
3. **Give each delegate everything**, because children get no history:
   - the product, file paths, and the taste guide above,
   - the screenshot folder,
   - the mockup page,
   - what's out of scope.
4. **Ask each delegate to reply in this format:**
   - `SCORE: x/10` for UX and polish,
   - `TOP FIXES` ranked `[must|should|nice] <where> <problem> → <change>`,
   - `KEEP`,
   - `SCOPE CREEP TO AVOID`.
5. **While the delegates run, check each finding against the code yourself**, but don't edit files a delegate is still reading.
   - Fix the ones you agree with, using the smallest change.
   - Decline scope creep in one line.
6. **Run a new round** with a ledger listing each finding as resolved, declined or still open, plus the new screenshots. Keep going until every member reports nothing left in scope. Jasiel's bar is **9.5**.
7. **Put a council table in the PR description** with each member's score for each round. If a member gave no score, write "no score". Don't post it as a comment.

**Last council**, the UX of the slim toasts and job dock:

| Member | Round 1 | Round 2 | Round 3 |
| --- | --- | --- | --- |
| Claude Opus | 8.5 | no score | 9.2 |
| GPT Sol | 8.3 | 8.9 | 9.2 |
| GPT Astra | 8.4 | 9.0 | 9.3 |

- **Verdict:** "the design is good; it needs targeted fixes, not a redesign." All the fixes are merged.
- **Gap to 9.5:** nobody has checked the toasts with the real **Onest** font (the render tool uses a stand-in), or tried the keyboard changes by hand. Do both before the next council.
- **Out of scope last time:** a wider dock, segmented bars, job trees, a new type system, and more animation.

For code changes, the separate **sol-loop** review (`~/.claude/skills/sol-loop`, gpt-6.1-sol on high, up to 3 rounds) was also used. "full stop" means: do the work end to end and open a PR, but don't merge.

## Tools for seeing UI changes

- **`tools/render-toasts/render.sh [out.png]`** renders `AgentToast.qml` offscreen, with stand-in Pond parts, to a PNG. It doesn't touch the desktop.
  - Edit the sample notifications in `tools/render-toasts/shell.qml`.
  - It needs `qs` (Quickshell) and Pond's files in `/usr/share/pond-interface/shell`.
  - Mascots use plain `Image` in the render only, because offscreen grabs drop `IconImage`/`ClippingRectangle`.
- **`tools/dock-demo.py working|needs|failed|clear`** puts fake job and agent sessions in the **live** dock so you can screenshot each state.
  - Always run `clear` afterwards.
  - Set `TOASTY_DEMO_JOB=<a real T3 thread id>` to fold the running orchestrator into the demo job.
- **Live screenshots** on niri: `grim full.png`, then crop the left column. The screen is 2256×1503 at 1.75 scale, and the agents card is at about x 10–310, y 480–1050.
- **Real T3 child agents:** `delegate_task` with `claude-haiku-5-5` on low effort and a `sleep 75` task brings up a real job row for about a minute.
- **What the dock currently shows:** `qs ipc -c pond-shell call toastyDock status` prints its rows as JSON.
- **Test toasts:**
  - `~/.local/bin/toasty-notify --agent Claude --kind done --context x --detach "Done in x" "message…"`
  - `toasty doctor --toast`

## Screenshots

In `screenshots/`:

| File | Shows |
| --- | --- |
| 00 | The original dock Jasiel loves (keep its base) |
| 01 | The slim toasts: Done with a reply, the sent echo, More expanded, Needs you with a command, Failed, a question with a reply |
| 02 | A usage-limit toast; "More" reveals the real error |
| 03 | A job working vs a job that needs you (staged) |
| 04 | The Jobs/Agents headings |
| 05 | A job's Failed outranking a working child |
| 06 | A dimmed paused mascot; dark text on "Needs you" |
| 07 | A live job row from real T3 child agents |

Shots 01 and 02 come from the offscreen render with a stand-in font.

## Open work

1. **Remove Allow/Deny from toasts.** Jasiel decided this, and the change is fully described:
   - **The permission hooks** (`toasty-claude-permission`, `toasty-codex-permission`) only notify and return right away.
   - **`toasty-notify`** drops the approval actions and the code that sends the answer back, but keeps replies.
   - **`AgentToast.qml`** drops the `actionRow` and stamp code for approvals.
   - **`PROTOCOL.md`** drops the action rows.
2. **More UI styles for the council**, which is this handoff's purpose. Start from the ideas list above. Keep the dock base and the slim toast feel.
3. Check the Onest font and keyboard use for real before claiming 9.5.
4. Smaller follow-ups:
   - Kitty replies need remote control in kitty.conf.
   - The docs PR to bring README, DESIGN and ROADMAP up to date with `main`.
   - Codex grouping in T3, if T3 ever passes the thread id to Codex hooks.

## Getting started

```sh
git clone git@github.com:001vamp/lighthouse.git && cd lighthouse   # or: git fetch origin
git checkout main
git fetch origin handoff-ui && git show origin/handoff-ui:handoff-ui/HANDOFF.md
git worktree add ../lighthouse-handoff origin/handoff-ui            # mockups, screenshots and tools, kept off main
toasty/tests/run                                                   # 25 tests OK
toasty/bin/toasty install --link                                   # only on a Pond machine
```
