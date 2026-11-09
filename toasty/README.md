# Toasty

Toasty shows a toast when an AI coding agent needs you. When an agent finishes, fails or asks permission, a small card appears. From the card you can:

- reply, and your reply is typed straight into the agent's terminal;
- Allow or Deny the command it wants to run;
- jump to the exact window or thread.

A dock shows each agent's live status: Working, Waiting, Done or Failed. Everything else stays quiet: Done toasts wait up to 3 seconds while you're typing, and after 5 idle minutes they wait for one "While you were away" card instead of piling up.

It works with **Claude Code** and **Codex**, in the terminal (kitty) and in their desktop apps. Other agents can be added through the [agent registry](share/agents.json).

## Install

```bash
git clone https://github.com/001vamp/lighthouse && cd lighthouse/toasty
bin/toasty install
toasty doctor --toast
```

`install` copies the helpers into `~/.local/bin` and adds Toasty's hooks to `~/.claude/settings.json`, `~/.codex/hooks.json` and `~/.codex/config.toml`. If you run Pond, it also adds Toasty's [shell client](shell/) to it and restarts Pond.

- It only ever adds its own entries. It backs up each file next to itself first (`*.toasty-backup-<time>`).
- It never replaces a Codex `notify` program you already use.
- `toasty uninstall` removes exactly what it added.

`toasty doctor` checks every piece and says what's missing and how to fix it.

**Codex:** the next time Codex starts (CLI or app), it asks you to review its new hooks. Choose **Trust all and continue**, or Toasty's Codex hooks won't run. Codex hooks run outside its sandbox, which is why it asks.

**Needs:** Python 3 with GObject (`python-gobject` on Arch, `python3-gi` on Debian/Ubuntu) and a notification server.

- **Replying from a toast:** needs kitty with remote control (`allow_remote_control socket-only` and `listen_on unix:@kitty` in kitty.conf).
- **Raising windows on click, and skipping toasts for windows you're already looking at:** needs niri today.

## The honest limits

- **The full experience needs a Toasty-aware shell.** Without one, toasts arrive as plain notifications that still have their buttons. Agent cards, the dock, the away card and the "Sent"/"Allowed" endings come from the Quickshell client in Pond, which is the reference shell. The [protocol](PROTOCOL.md) is open, so other shells can implement it.
- **Replies only reach agents running in kitty.** The desktop apps don't offer a way in. Their toasts open the exact thread instead.
- **Codex:** no "Always allow" (Codex doesn't accept rule changes from hooks yet) and no Failed state (Codex sends no failure event). While a Codex permission toast is up, Codex shows nothing in its terminal. So the toast steps aside as soon as you look at Codex, and Codex's own prompt takes over.
- **Sticky cards go first.** Only one card shows at a time, so a Done toast waits behind an unanswered Allow/Deny card. The dock shows it finished straight away.
- **Desktop apps send their own notification too.** Toasty merges one that arrives within 3 seconds of its toast for the same agent into that card, so clicking the card uses the app's own link. Two sessions finishing at the same moment can merge the wrong pair.
- **An app having focus can't say which session you're looking at.** A toast is only skipped for a focused app when that app has no other session active in the last hour.
- **Codex turns the app runs out of sight** (like ChatGPT's suggestions) are left out: they have no saved thread to show or open.
- **Terminal status is read from window titles,** so a CLI update that changes its title can break it. The patterns live in `agents.json`.

## Adding an agent

Add an entry to `share/agents.json`. That's the whole change for toasts and the dock. Fill in:

- `name` and `aliases`;
- `accent` and `mascot`;
- `command`, the program name that replies may be typed into;
- `title` patterns for working, waiting and idle;
- `can`: what its hooks support.

Then have the agent's hook call `toasty-notify` (see `toasty-notify --help`) and, for desktop apps, write its state to `sessions.json` ([PROTOCOL.md](PROTOCOL.md)). `bin/toasty-codex-*` is a complete small example.

## Development

```bash
tests/run                 # private D-Bus session, fake notification server, fake kitty and niri
bin/toasty install --link # use this checkout directly
```

## License

Personal project, no license chosen. The [shell client](shell/) includes files from Pond, which is under BSD 3-Clause, © Butterfly Interaction Inc.
