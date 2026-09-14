# Agent harness integrations

Tool-call integrations that let an agent harness query code-index-cli
directly, instead of an agent shelling out to `code-index search
--format json` itself. Each subdirectory targets one harness.

| Harness | Path | What it adds |
| --- | --- | --- |
| [pi](pi/) | `integrations/pi/` | A `pi.registerTool()` extension: `code_index_search` (scoped to one project) + `code_index_projects` (lists configured projects) |
| [Claude Code](claude-code/) | `integrations/claude-code/` | A Skill (`code-index-search`) teaching Claude to run `code-index search --format json` via Bash — Claude Code has no direct tool-registration API, so this is the equivalent given a Skill's more limited reach |

Adding a new harness: give it its own subdirectory here (e.g.
`integrations/claude-code/`) with its own README covering install and
any harness-specific conventions. These all assume `code-index` is
already installed and configured — see the main [README](../README.md)
— and, for multi-project setups, the project list from
[`examples/multi-project-watch/`](../examples/multi-project-watch/).
