#!/usr/bin/env bash
# Starts `code-index watch` for every project listed in projects.json,
# each as a background process with its own log + pid file. Safe to
# re-run: projects already running are left alone.
set -euo pipefail

CONF_DIR="$HOME/.config/code-index"
CONF="$CONF_DIR/projects.json"
RUN_DIR="$CONF_DIR/run"
LOG_DIR="$CONF_DIR/logs"

mkdir -p "$RUN_DIR" "$LOG_DIR"

if [[ ! -f "$CONF" ]]; then
  echo "No project list at $CONF" >&2
  exit 1
fi

command -v code-index >/dev/null || { echo "code-index not found on PATH" >&2; exit 1; }

python3 -c 'import json,sys; [print(p) for p in json.load(open(sys.argv[1]))]' "$CONF" |
while IFS= read -r path; do
  [[ -d "$path" ]] || { echo "skip (missing dir): $path" >&2; continue; }
  name=$(basename "$path")
  pidfile="$RUN_DIR/$name.pid"

  if [[ -f "$pidfile" ]] && kill -0 "$(cat "$pidfile")" 2>/dev/null; then
    echo "already running: $name ($(cat "$pidfile"))"
    continue
  fi

  echo "starting watch: $name  ($path)"
  nohup code-index watch -w "$path" >> "$LOG_DIR/$name.log" 2>&1 &
  echo $! > "$pidfile"
done
