#!/usr/bin/env bash
# Stops Option-A watchers started by watch-all.sh (SIGINT, so the hash
# cache flushes cleanly) and clears their pid files. With no argument,
# stops every pidfile found (regardless of whether it's still listed
# in projects.json, to also clean up stragglers). Pass a group name to
# scope to just that group's currently-listed projects, e.g.
# `stop-all.sh work`.
#
# Doesn't touch LaunchAgent-managed watchers — use
# uninstall-launchagents.sh for those. A listed project with no pidfile
# whose watcher is launchd-managed is skipped with a warning on stderr
# (stdout stays clean; install-launchagents.sh calls this script with
# stderr suppressed, so its internal stop doesn't echo these).
set -euo pipefail

CONF_DIR="$HOME/.config/code-index"
CONF="$CONF_DIR/projects.json"
RUN_DIR="$CONF_DIR/run"
GROUP="${1:-}"

stop_one() {
  local name="$1" pidfile="$RUN_DIR/$1.pid" label_prefix="${2:-}"

  if [[ ! -f "$pidfile" ]]; then
    # No Option-A watcher to stop. If a LaunchAgent owns this project,
    # warn instead of returning silently: status-all.sh reports both
    # modes, so it's easy to assume this script stops both — it doesn't,
    # uninstall-launchagents.sh does. Same launchctl probe watch-all.sh
    # uses; macOS only, harmless no-op where launchctl doesn't exist.
    if command -v launchctl >/dev/null 2>&1 && launchctl list "com.code-index.watch.$name" >/dev/null 2>&1; then
      echo "skipped: ${label_prefix}${name} is launchd-managed — use uninstall-launchagents.sh to stop it" >&2
    fi
    return 0
  fi
  local pid
  pid=$(cat "$pidfile")
  if kill -0 "$pid" 2>/dev/null; then
    echo "stopping: ${label_prefix}${name} (pid $pid)"
    kill -INT "$pid"
  else
    echo "not running: ${label_prefix}${name} (stale pid file)"
  fi
  rm -f "$pidfile"
}

if [[ -n "$GROUP" ]]; then
  # Captured, not process-substituted — see watch-all.sh for why.
  resolved=$(python3 "$CONF_DIR/scripts/resolve_projects.py" "$CONF" "$GROUP")
  if [[ -n "$resolved" ]]; then
    while IFS=$'\t' read -r group name path; do
      stop_one "$name" "[$group] "
    done <<<"$resolved"
  fi
else
  shopt -s nullglob
  for pidfile in "$RUN_DIR"/*.pid; do
    stop_one "$(basename "$pidfile" .pid)"
  done
fi
