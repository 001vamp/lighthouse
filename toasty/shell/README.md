# Toasty shell client

The Quickshell side of Toasty: agent toasts, the dock row, the away card, and the mascots. It runs inside [Pond](https://github.com/bflycomputer/pond-interface), a Quickshell shell.

- `pond.patch` is Toasty's changes to Pond's own files: the notification state, the toast window, the notification card and the sidebar.
- `files/` holds Toasty's new files, laid out like Pond's `shell/` folder: the agent toast and buttons, the dock card, and the mascots.

`toasty install` puts these into the Pond you're running:

- **Pond from a package** (`/usr/share/pond-interface`, run as `qs -c pond-shell`): it copies the packaged shell to `~/.config/quickshell/pond-shell`, adds `files/`, and applies `pond.patch`. Quickshell loads that copy instead of the packaged one, at login too. If the patch doesn't fit, nothing is changed. After a Pond update, `toasty doctor` says so; run `toasty install` again to rebuild on the new Pond.
- **A Pond checkout in `~/.local/share/pond-interface`** that already has Toasty: left as it is.

Install and uninstall restart Pond for you. They stop the running Pond by its instance id, because `qs kill -c pond-shell` would look for the copy that was just added or removed, not the one that's running.

A rebuild replaces that copy; if it was edited by hand, the old one is kept beside it as `pond-shell.toasty-backup-<time>`.

The patch is made against `pond-shell-omarchy` 0.1.11. A Pond checkout with Toasty in it doesn't get changes to this patch automatically; apply them there too. Pond is under BSD 3-Clause, © Butterfly Interaction Inc. ([LICENSE-pond](LICENSE-pond)).
