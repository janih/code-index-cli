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
cp examples/multi-project-watch/*.sh ~/.config/code-index/scripts/
chmod +x ~/.config/code-index/scripts/*.sh
cp examples/multi-project-watch/projects.json.example ~/.config/code-index/projects.json
```

Edit `~/.config/code-index/projects.json` to list your project paths:

```json
[
  "/path/to/project-a",
  "/path/to/project-b"
]
```

## Option A — plain background processes

```sh
~/.config/code-index/scripts/watch-all.sh    # start (skips already-running projects)
~/.config/code-index/scripts/status-all.sh   # running/stopped + last log line, per project
~/.config/code-index/scripts/stop-all.sh     # SIGINT each one so its cache flushes cleanly
```

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

`install-launchagents.sh` stops anything started via `watch-all.sh` first.
**Don't run both options for the same project** — two watchers on one
workspace will race over the same hash cache file.

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
