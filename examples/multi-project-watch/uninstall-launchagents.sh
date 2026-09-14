#!/usr/bin/env bash
# macOS only. Stops and removes code-index LaunchAgents. With no
# argument, removes every one installed by install-launchagents.sh.
# Pass a group name to remove just that group's agents and leave
# everything else running, e.g. `uninstall-launchagents.sh work`.
set -euo pipefail

CONF_DIR="$HOME/.config/code-index"
CONF="$CONF_DIR/projects.json"
AGENTS_DIR="$HOME/Library/LaunchAgents"
PREFIX="com.code-index.watch."
UID_GUI="gui/$(id -u)"
GROUP="${1:-}"

remove_one() {
  local label="$1"
  local plist="$AGENTS_DIR/${label}.plist"
  [[ -e "$plist" ]] || return 0
  echo "stopping + removing: $label"
  launchctl bootout "$UID_GUI/$label" >/dev/null 2>&1 || true
  rm -f "$plist"
}

if [[ -n "$GROUP" ]]; then
  # Captured, not process-substituted — see watch-all.sh for why.
  resolved=$(python3 "$CONF_DIR/scripts/resolve_projects.py" "$CONF" "$GROUP")
  if [[ -n "$resolved" ]]; then
    while IFS=$'\t' read -r group name path; do
      remove_one "${PREFIX}${name}"
    done <<<"$resolved"
  fi
else
  shopt -s nullglob
  for plist in "$AGENTS_DIR"/${PREFIX}*.plist; do
    remove_one "$(basename "$plist" .plist)"
  done
fi
