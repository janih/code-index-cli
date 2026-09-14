#!/usr/bin/env bash
# Stops every code-index watcher started by watch-all.sh (SIGINT, so the
# hash cache flushes cleanly), and clears the pid files.
set -euo pipefail

RUN_DIR="$HOME/.config/code-index/run"
shopt -s nullglob
for pidfile in "$RUN_DIR"/*.pid; do
  name=$(basename "$pidfile" .pid)
  pid=$(cat "$pidfile")
  if kill -0 "$pid" 2>/dev/null; then
    echo "stopping: $name (pid $pid)"
    kill -INT "$pid"
  else
    echo "not running: $name (stale pid file)"
  fi
  rm -f "$pidfile"
done
