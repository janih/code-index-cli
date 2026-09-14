# Multi-project watch

`code-index watch` targets exactly one `--workspace` and runs in the
foreground — there's no built-in way to watch several unrelated projects
at once. This is a small wrapper pattern around a list of project paths
that starts one `watch` process per project.

It pairs well with a [global config](../../README.md#configuration-layering)
at `~/.config/code-index/config.json` holding the shared embedder + Qdrant
settings: once that's set, adding a project here needs no per-project
`.code-index.json` at all.

## Setup

```sh
mkdir -p ~/.config/code-index/scripts
for f in examples/multi-project-watch/*.sh; do
  ln -sf "$(pwd)/$f" ~/.config/code-index/scripts/"$(basename "$f")"
done
[[ -f ~/.config/code-index/projects.json ]] || cp examples/multi-project-watch/projects.json.example ~/.config/code-index/projects.json
```

Symlinked, not copied, so a fix to a script here (like the one below) takes
effect the next time you run it — no re-copy step to remember. Use a plain
`cp` instead only if you want a version pinned independent of this repo's
working tree.

Edit `~/.config/code-index/projects.json` to list your project paths:

```json
[
  "/path/to/project-a",
  "/path/to/project-b"
]
```

**Pick one mode per machine and stick with it.** Two watchers on the same
workspace race over the same hash cache file — the concrete failure mode
this bit in practice: adopt Option B, then later add a project by
re-running Option A's `watch-all.sh` out of habit, which starts a second,
untracked watcher for every project *already* under launchd. `watch-all.sh`
now refuses to do that (it checks for a matching LaunchAgent first and
skips with a message instead), but there's no equivalent guard the other
way — running `install-launchagents.sh` while a project has a manual
Option A watcher is fine (it stops those first), the risk is only ever
"Option A script started after Option B already owns a project."

## Option A — plain background processes

```sh
~/.config/code-index/scripts/watch-all.sh    # start (skips already-running/launchd-managed projects)
~/.config/code-index/scripts/status-all.sh   # running/launchd/stopped + last log line + its age, per project
~/.config/code-index/scripts/stop-all.sh     # SIGINT each one so its cache flushes cleanly
```

`status-all.sh` checks both modes (it'll report `launchd (pid ...)` for a
project running under Option B), because reporting "stopped" next to a
log line from a process that's actually alive under launchd — just quiet
— reads as a contradiction. The log tail itself can't tell you that on
its own either: `code-index watch` only prints on startup or when a
batch is processed, so an idle project's last line can be hours old
whether the watcher is alive-and-waiting or long dead — hence the `[Xm
ago]` age on it, and hence checking real process state rather than
trusting the log's content to imply anything about the present.

Logs land at `~/.config/code-index/logs/<project-name>.log`, pid files at
`~/.config/code-index/run/<project-name>.pid`. Re-run `watch-all.sh`
after adding a project to the list. This mode doesn't survive logout —
run it again after each reboot, or use Option B.

## Option B — macOS LaunchAgents (start at login, restart on crash)

```sh
~/.config/code-index/scripts/install-launchagents.sh    # generate + load one agent per project
~/.config/code-index/scripts/status-launchd.sh           # PID + exit status per agent
~/.config/code-index/scripts/uninstall-launchagents.sh   # stop + remove all of them
```

Each project gets `~/Library/LaunchAgents/com.code-index.watch.<name>.plist`
with `RunAtLoad` and `KeepAlive` set, so it starts at login and restarts
itself if it exits (throttled to 30s so a persistent failure, e.g. Qdrant
being down, doesn't spin-loop). Re-run `install-launchagents.sh` after
editing `projects.json` — it also removes agents for projects no longer
listed.

`install-launchagents.sh` stops anything started via `watch-all.sh` first
(the reverse case — see the warning above).

Once a project is under launchd, stop/restart it with `launchctl`, not
`kill` — a raw `kill` just gets it restarted by `KeepAlive`:

```sh
launchctl bootout gui/$(id -u)/com.code-index.watch.<name>
```

## Updating the `code-index` binary

The plist points at a binary *path*, not a version. Reinstalling
(`cargo install --path .`, or replacing the binary another way) doesn't
disturb an already-running watcher — it keeps executing the old build
until it's restarted (the OS keeps a running process mapped to the old
file even after the path is overwritten). `KeepAlive` won't trigger this
on its own since a rebuild doesn't make the process exit.

To move a watcher onto a newly installed binary, restart it explicitly:

```sh
launchctl kickstart -k gui/$(id -u)/com.code-index.watch.<name>   # one agent
~/.config/code-index/scripts/install-launchagents.sh              # all of them
```

(Option A processes need the equivalent: `stop-all.sh` then `watch-all.sh`.)

If the update also bumps the index-format constants (block limits, the
Qdrant point-id namespace — see `AGENTS.md`), a restart alone isn't
enough; that orphans the existing collection regardless of how `watch` is
supervised, and needs `code-index clear && code-index index` per project.

## Caveats

- These scripts assume `code-index` is on `PATH` (`install-launchagents.sh`
  resolves and embeds its absolute path at install time — re-run it if you
  reinstall the binary elsewhere).
- Uses `python3` for JSON parsing (no `jq` dependency).
- LaunchAgents are macOS-specific; Option A is portable to any Unix.
