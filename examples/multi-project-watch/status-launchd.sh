#!/usr/bin/env bash
# macOS only. Shows the launchd status (PID + last exit code) of every
# installed code-index watcher agent.
set -euo pipefail
PREFIX="com.code-index.watch."
launchctl list | awk -v p="$PREFIX" 'NR==1 || index($3,p)' || true
