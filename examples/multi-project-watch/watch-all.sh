#!/usr/bin/env bash
# Starts `code-index watch` for every project listed in projects.json,
# each as a background process with its own log + pid file. Safe to
# re-run: projects already running (under this script or a
# LaunchAgent) are left alone.
#
# Pass a group name to scope to just that group, e.g. `watch-all.sh work`.
set -euo pipefail

CONF_DIR="$HOME/.config/code-index"
CONF="$CONF_DIR/projects.json"
RUN_DIR="$CONF_DIR/run"
LOG_DIR="$CONF_DIR/logs"
GROUP="${1:-}"

mkdir -p "$RUN_DIR" "$LOG_DIR"

if [[ ! -f "$CONF" ]]; then
  echo "No project list at $CONF" >&2
  exit 1
fi

command -v code-index >/dev/null || { echo "code-index not found on PATH" >&2; exit 1; }

# Captured (not piped/process-substituted) so an unknown-group failure
# from resolve_projects.py actually exits this script nonzero under
# `set -e` — a process substitution's exit status isn't seen by it.
resolved=$(python3 "$CONF_DIR/scripts/resolve_projects.py" "$CONF" "$GROUP")
[[ -z "$resolved" ]] && exit 0

while IFS=$'\t' read -r group name path; do
  [[ -d "$path" ]] || { echo "skip (missing dir): $path" >&2; continue; }
  pidfile="$RUN_DIR/$name.pid"

  # Refuse to start a second watcher for a project already managed by a
  # LaunchAgent (install-launchagents.sh) — this pidfile-only check can't
  # see those, and two watchers on one workspace race over the same hash
  # cache file. macOS only; harmless no-op where launchctl doesn't exist.
  if command -v launchctl >/dev/null 2>&1 && launchctl list "com.code-index.watch.$name" >/dev/null 2>&1; then
    echo "already managed by launchd: [$group] $name — use install-launchagents.sh instead, skipping" >&2
    continue
  fi

  if [[ -f "$pidfile" ]] && kill -0 "$(cat "$pidfile")" 2>/dev/null; then
    echo "already running: [$group] $name ($(cat "$pidfile"))"
    continue
  fi

  echo "starting watch: [$group] $name  ($path)"
  nohup code-index watch -w "$path" >> "$LOG_DIR/$name.log" 2>&1 &
  echo $! > "$pidfile"
done <<<"$resolved"
