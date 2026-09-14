# code-index for Claude Code

Claude Code has no `pi.registerTool()` equivalent — there's no first-party
way to register a brand-new callable tool with its own JSON-schema
parameters short of running an **MCP server**. The lighter-weight
mechanism is a **Skill**: a packaged Markdown instructions file that
teaches Claude *when* and *how* to use its existing tools (here, `Bash`)
for a task, rather than defining a new one.

This integration is a Skill, not an MCP server: `code-index search
--format json` already gives clean, structured output over stdout, so
there's nothing an MCP server would add here beyond what a well-written
Skill plus `Bash` already does — and a Skill needs no server process to
keep running. Revisit MCP if code-index-cli grows a use case an
instructions file can't cover (e.g. long-lived state across calls,
streaming, or exposing it to a host with no shell tool at all).

## Install

Claude Code auto-discovers skills from `~/.claude/skills/*/SKILL.md`.
Symlink the skill directory in so edits here take effect without a copy
step (matches how `integrations/pi/` is installed):

```sh
ln -sf "$(pwd)/integrations/claude-code/skills/code-index-search" ~/.claude/skills/code-index-search
```

Or copy it if you want Claude Code running a version pinned independent
of this repo's working tree:

```sh
cp -r integrations/claude-code/skills/code-index-search ~/.claude/skills/
```

For a single project instead of globally, put (or symlink) it at
`.claude/skills/code-index-search/` inside that project's own repo.

No reload step — skills are read fresh each session.

## What it teaches

- Reading `~/.config/code-index/projects.json` to map a project name to
  its absolute path (see
  [`examples/multi-project-watch/`](../../examples/multi-project-watch/)).
- Running `code-index search -w <path> -q "<query>" --format json`
  and parsing the result shape.
- The workspace-path-mismatch pitfall (`-w .` vs `-w <absolute path>`
  are different, unrelated Qdrant collections — the single most common
  way this returns "0 results" on an actually-indexed project).

## Verifying it loaded

```sh
claude -p 'Use the code-index-search skill: read ~/.config/code-index/projects.json and tell me what projects are configured.'
```

Claude should read the file via its own tools and list the projects —
there's no new tool call to look for, since a Skill doesn't add one.
