#!/usr/bin/env bash
# Shows which listed projects have a live watcher, plus the last log line.
set -euo pipefail

CONF="$HOME/.config/code-index/projects.json"
RUN_DIR="$HOME/.config/code-index/run"
LOG_DIR="$HOME/.config/code-index/logs"

python3 -c 'import json,sys; [print(p) for p in json.load(open(sys.argv[1]))]' "$CONF" |
while IFS= read -r path; do
  name=$(basename "$path")
  pidfile="$RUN_DIR/$name.pid"
  logfile="$LOG_DIR/$name.log"

  if [[ -f "$pidfile" ]] && kill -0 "$(cat "$pidfile")" 2>/dev/null; then
    state="running (pid $(cat "$pidfile"))"
  else
    state="stopped"
  fi

  last=""
  [[ -f "$logfile" ]] && last=$(tail -n 1 "$logfile" 2>/dev/null || true)
  printf "%-10s %-30s %s\n" "$state" "$name" "$last"
done
