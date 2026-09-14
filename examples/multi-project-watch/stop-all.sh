#!/usr/bin/env bash
# Stops Option-A watchers started by watch-all.sh (SIGINT, so the hash
# cache flushes cleanly) and clears their pid files. With no argument,
# stops every pidfile found (regardless of whether it's still listed
# in projects.json, to also clean up stragglers). Pass a group name to
# scope to just that group's currently-listed projects, e.g.
# `stop-all.sh work`.
#
# Doesn't touch LaunchAgent-managed watchers — use
# uninstall-launchagents.sh for those.
set -euo pipefail

CONF_DIR="$HOME/.config/code-index"
CONF="$CONF_DIR/projects.json"
RUN_DIR="$CONF_DIR/run"
GROUP="${1:-}"

stop_one() {
  local name="$1" pidfile="$RUN_DIR/$1.pid" label_prefix="${2:-}"
  [[ -f "$pidfile" ]] || return 0
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
