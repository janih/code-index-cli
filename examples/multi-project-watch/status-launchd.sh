#!/usr/bin/env bash
# macOS only. Shows the launchd status (PID + last exit code) of every
# installed code-index watcher agent. Pass a group name to scope to
# just that group's agents, e.g. `status-launchd.sh work`.
set -euo pipefail

CONF_DIR="$HOME/.config/code-index"
CONF="$CONF_DIR/projects.json"
PREFIX="com.code-index.watch."
GROUP="${1:-}"

if [[ -n "$GROUP" && -f "$CONF" ]]; then
  # Captured, not process-substituted, so an unknown group actually
  # exits this script nonzero under `set -e`.
  resolved=$(python3 "$CONF_DIR/scripts/resolve_projects.py" "$CONF" "$GROUP")
  launchctl list | head -1
  if [[ -n "$resolved" ]]; then
    while IFS=$'\t' read -r group name path; do
      launchctl list | awk -v want="${PREFIX}${name}" '$3 == want'
    done <<<"$resolved"
  fi
else
  launchctl list | awk -v p="$PREFIX" 'NR==1 || index($3,p)' || true
fi
