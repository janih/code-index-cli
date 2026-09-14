#!/usr/bin/env bash
# macOS only. Stops and removes every code-index LaunchAgent installed
# by install-launchagents.sh.
set -euo pipefail

AGENTS_DIR="$HOME/Library/LaunchAgents"
PREFIX="com.code-index.watch."
UID_GUI="gui/$(id -u)"

shopt -s nullglob
for plist in "$AGENTS_DIR"/${PREFIX}*.plist; do
  label=$(basename "$plist" .plist)
  echo "stopping + removing: $label"
  launchctl bootout "$UID_GUI/$label" >/dev/null 2>&1 || true
  rm -f "$plist"
done
