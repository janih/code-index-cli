#!/usr/bin/env bash
# Shows which listed projects have a live watcher -- checking both a
# plain Option A process (pidfile) and, on macOS, an Option B
# LaunchAgent -- plus the last log line and how long ago it was
# written. The two modes share one log file per project (see
# install-launchagents.sh), and code-index watch only prints on
# startup or when a batch is processed, so a quiet project's log tail
# can be hours old even while the watcher is alive and fine; the age
# is there so that doesn't read as "it died right after printing that".
#
# Pass a group name to scope to just that group, e.g. `status-all.sh work`.
set -euo pipefail

CONF_DIR="$HOME/.config/code-index"
CONF="$CONF_DIR/projects.json"
RUN_DIR="$CONF_DIR/run"
LOG_DIR="$CONF_DIR/logs"
GROUP="${1:-}"

age() {
  local mtime now delta
  mtime=$(stat -f %m "$1" 2>/dev/null || stat -c %Y "$1" 2>/dev/null) || { echo "?"; return; }
  now=$(date +%s)
  delta=$(( now - mtime ))
  if   (( delta < 60 ));    then echo "${delta}s ago"
  elif (( delta < 3600 ));  then echo "$(( delta / 60 ))m ago"
  elif (( delta < 86400 )); then echo "$(( delta / 3600 ))h ago"
  else echo "$(( delta / 86400 ))d ago"
  fi
}

# Captured, not process-substituted — see watch-all.sh for why.
resolved=$(python3 "$CONF_DIR/scripts/resolve_projects.py" "$CONF" "$GROUP")
[[ -z "$resolved" ]] && exit 0

while IFS=$'\t' read -r group name path; do
  pidfile="$RUN_DIR/$name.pid"
  logfile="$LOG_DIR/$name.log"

  if [[ -f "$pidfile" ]] && kill -0 "$(cat "$pidfile")" 2>/dev/null; then
    state="running (pid $(cat "$pidfile"))"
  elif command -v launchctl >/dev/null 2>&1; then
    ll_out=$(launchctl list "com.code-index.watch.$name" 2>/dev/null || true)
    if [[ -n "$ll_out" ]]; then
      ll_pid=$(awk -F'= ' '/"PID"/{gsub(";","",$2); print $2}' <<<"$ll_out")
      if [[ -n "$ll_pid" ]]; then state="launchd (pid $ll_pid)"; else state="launchd (loaded, not running)"; fi
    else
      state="stopped"
    fi
  else
    state="stopped"
  fi

  last=""
  logage=""
  if [[ -f "$logfile" ]]; then
    last=$(tail -n 1 "$logfile" 2>/dev/null || true)
    logage=" [$(age "$logfile")]"
  fi
  printf "%-26s %-10s %-30s %s%s\n" "$state" "[$group]" "$name" "$last" "$logage"
done <<<"$resolved"
