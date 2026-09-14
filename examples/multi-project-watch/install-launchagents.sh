#!/usr/bin/env bash
# macOS only. Generates and (re)loads a LaunchAgent per project in
# projects.json, so `code-index watch` runs continuously: starts at
# login, restarts on crash (throttled to avoid a tight restart loop if
# e.g. Qdrant is down).
#
# Re-run after editing projects.json to pick up added/removed projects
# (agents for paths no longer listed are stopped and removed).
set -euo pipefail

CONF_DIR="$HOME/.config/code-index"
CONF="$CONF_DIR/projects.json"
LOG_DIR="$CONF_DIR/logs"
AGENTS_DIR="$HOME/Library/LaunchAgents"
PREFIX="com.code-index.watch."
UID_GUI="gui/$(id -u)"

mkdir -p "$LOG_DIR" "$AGENTS_DIR"

BINARY=$(command -v code-index) || { echo "code-index not found on PATH" >&2; exit 1; }

# Stop any watcher started manually by watch-all.sh first — otherwise
# launchd's copy and the manual one would both watch the same workspace.
"$CONF_DIR/scripts/stop-all.sh" 2>/dev/null || true

installed_labels=()
while IFS= read -r path; do
  [[ -d "$path" ]] || { echo "skip (missing dir): $path" >&2; continue; }
  name=$(basename "$path")
  label="${PREFIX}${name}"
  plist="$AGENTS_DIR/${label}.plist"
  log="$LOG_DIR/${name}.log"
  installed_labels+=("$label")

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
  echo "installed + started: $label"
done < <(python3 -c 'import json,sys; [print(p) for p in json.load(open(sys.argv[1]))]' "$CONF")

# Remove agents for projects no longer in projects.json.
shopt -s nullglob
for plist in "$AGENTS_DIR"/${PREFIX}*.plist; do
  label=$(basename "$plist" .plist)
  found=0
  for l in "${installed_labels[@]:-}"; do [[ "$l" == "$label" ]] && found=1; done
  if [[ "$found" -eq 0 ]]; then
    echo "removing stale agent: $label"
    launchctl bootout "$UID_GUI/$label" >/dev/null 2>&1 || true
    rm -f "$plist"
  fi
done
