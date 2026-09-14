# code-index for pi

A [pi](https://github.com/earendil-works/pi) extension exposing two tools
so an agent can query code-index-cli directly instead of shelling out to
`code-index search --format json` itself:

- **`code_index_search`** — semantic search scoped to one project
  (`project` name, or a raw `workspace` path), plus `query`, optional
  `directory` and `limit`.
- **`code_index_projects`** — lists the projects currently configured in
  `~/.config/code-index/projects.json` (see
  [`examples/multi-project-watch/`](../../examples/multi-project-watch/)).

## Install

pi auto-discovers extensions from `~/.pi/agent/extensions/`. Symlink
this file in so edits here take effect without a manual copy step:

```sh
ln -sf "$(pwd)/integrations/pi/code-index.ts" ~/.pi/agent/extensions/code-index.ts
```

(Prefer a plain copy instead if you want pi running an older, pinned
version independent of this repo's working tree.)

New sessions pick it up immediately — no `/reload` needed. If you add or
remove a project in `projects.json` while pi is already running, the
`project` parameter description (the "known projects" hint baked into
the tool schema at load time) goes stale, but `code_index_projects` and
the actual validation always read the file fresh. Run `/reload-runtime`
(or restart pi) to refresh the stale hint text.

## Requirements

- `code-index` installed and on `PATH`, or at `~/.cargo/bin/code-index`,
  `/opt/homebrew/bin/code-index`, or `/usr/local/bin/code-index` (checked
  in that order — pi's process may not inherit an interactive shell's
  `PATH`, the same issue documented in
  [`examples/multi-project-watch/`](../../examples/multi-project-watch/)
  for launchd).
- At least one project configured — either via
  `~/.config/code-index/projects.json` (for the `project` parameter) or
  queried directly by absolute path via `workspace`.

## Verifying it loaded

```sh
pi -e integrations/pi/code-index.ts -p "call code_index_projects" --mode json
```

Look for a `tool_execution_end` event for `code_index_projects` in the
output.
