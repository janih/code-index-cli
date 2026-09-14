#!/usr/bin/env python3
"""Resolves projects.json into group<TAB>name<TAB>path lines, one per
project, optionally filtered to a single group.

projects.json is either:
  - a flat array of paths (treated as one implicit group, "default" --
    this is the pre-grouping format, kept working for compatibility), or
  - an object of {group_name: [paths, ...]}.

Usage: resolve_projects.py <projects.json> [group]
With no group, every project across every group is printed.
"""
import json
import os
import sys


def load_groups(conf_path):
    with open(conf_path) as f:
        data = json.load(f)
    if isinstance(data, list):
        return {"default": data}
    if isinstance(data, dict):
        return data
    raise ValueError(f"{conf_path}: expected a JSON array or object")


def main():
    if len(sys.argv) < 2:
        print("usage: resolve_projects.py <projects.json> [group]", file=sys.stderr)
        sys.exit(2)
    conf_path = sys.argv[1]
    # An empty string (a caller passing along an unset $GROUP) means the
    # same as not passing one at all.
    wanted_group = sys.argv[2] if len(sys.argv) > 2 and sys.argv[2] else None

    try:
        groups = load_groups(conf_path)
    except (OSError, ValueError, json.JSONDecodeError) as err:
        print(f"error: {err}", file=sys.stderr)
        sys.exit(1)

    # A name is reused if the same basename appears under two different
    # paths anywhere in the file -- the pidfile, log file and
    # LaunchAgent label are all keyed by name alone (not name+group), so
    # such a collision silently makes two projects fight over one slot
    # regardless of which group(s) a given run targets.
    seen = {}
    for paths in groups.values():
        for p in paths:
            name = os.path.basename(p)
            if name in seen and seen[name] != p:
                print(
                    f"warning: project name '{name}' used by both {seen[name]} "
                    f"and {p} -- names must be unique across all groups",
                    file=sys.stderr,
                )
            seen[name] = p

    if wanted_group is not None:
        if wanted_group not in groups:
            available = ", ".join(sorted(groups)) or "(none)"
            print(f"Unknown group '{wanted_group}'. Available: {available}", file=sys.stderr)
            sys.exit(1)
        groups = {wanted_group: groups[wanted_group]}

    for group, paths in groups.items():
        for p in paths:
            print(f"{group}\t{os.path.basename(p)}\t{p}")


if __name__ == "__main__":
    main()
