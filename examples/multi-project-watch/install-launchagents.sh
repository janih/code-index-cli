#!/usr/bin/env bash
# macOS only. Generates and (re)loads a LaunchAgent per project in
# projects.json, so `code-index watch` runs continuously: starts at
# login, restarts on crash (throttled to avoid a tight restart loop if
# e.g. Qdrant is down).
#
# Pass a group name to scope to just that group, e.g.
# `install-launchagents.sh work` — only that group's agents are
# created/refreshed; other groups' agents are left alone even though
# this run doesn't touch them.
#
# Re-run after editing projects.json to pick up added/removed
# projects. Agents for a project name no longer listed ANYWHERE in
# projects.json (any group) are stopped and removed, regardless of
# which group this run was scoped to.
set -euo pipefail

CONF_DIR="$HOME/.config/code-index"
CONF="$CONF_DIR/projects.json"
LOG_DIR="$CONF_DIR/logs"
AGENTS_DIR="$HOME/Library/LaunchAgents"
PREFIX="com.code-index.watch."
UID_GUI="gui/$(id -u)"
GROUP="${1:-}"

mkdir -p "$LOG_DIR" "$AGENTS_DIR"

BINARY=$(command -v code-index) || { echo "code-index not found on PATH" >&2; exit 1; }

# Stop any watcher started manually by watch-all.sh first, scoped the
# same way — otherwise launchd's copy and the manual one would both
# watch the same workspace.
"$CONF_DIR/scripts/stop-all.sh" "$GROUP" 2>/dev/null || true

# Captured, not process-substituted — see watch-all.sh for why.
resolved=$(python3 "$CONF_DIR/scripts/resolve_projects.py" "$CONF" "$GROUP")

if [[ -n "$resolved" ]]; then
  while IFS=$'\t' read -r group name path; do
    [[ -d "$path" ]] || { echo "skip (missing dir): $path" >&2; continue; }
    label="${PREFIX}${name}"
    plist="$AGENTS_DIR/${label}.plist"
    log="$LOG_DIR/${name}.log"

    cat > "$plist" <<EOF
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
    <key>Label</key><string>${label}</string>
    <key>ProgramArguments</key>
    <array>
        <string>${BINARY}</string>
        <string>watch</string>
        <string>-w</string>
        <string>${path}</string>
    </array>
    <key>RunAtLoad</key><true/>
    <key>KeepAlive</key><true/>
    <key>ThrottleInterval</key><integer>30</integer>
    <key>StandardOutPath</key><string>${log}</string>
    <key>StandardErrorPath</key><string>${log}</string>
</dict>
</plist>
EOF

    launchctl bootout "$UID_GUI/$label" >/dev/null 2>&1 || true
    launchctl bootstrap "$UID_GUI" "$plist"
    echo "installed + started: [$group] $label"
  done <<<"$resolved"
fi

# Remove agents for projects no longer listed anywhere in projects.json
# (checked against the full file, unscoped, even on a scoped run — a
# project outside the requested group is not "no longer listed").
all_labels=$(python3 "$CONF_DIR/scripts/resolve_projects.py" "$CONF" | cut -f2 | sed "s/^/${PREFIX}/")
shopt -s nullglob
for plist in "$AGENTS_DIR"/${PREFIX}*.plist; do
  label=$(basename "$plist" .plist)
  if ! grep -qxF "$label" <<<"$all_labels"; then
    echo "removing stale agent: $label"
    launchctl bootout "$UID_GUI/$label" >/dev/null 2>&1 || true
    rm -f "$plist"
  fi
done
