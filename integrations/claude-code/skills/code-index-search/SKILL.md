---
name: code-index-search
description: This skill should be used when the user asks to "search the codebase", "semantic search", "find where X is implemented", "how does <project> do Y", or asks a conceptual question about a project indexed by code-index-cli — especially when several unrelated projects are on the machine and the search must be scoped to one of them by name.
---

# code-index-search

Search one project's codebase with semantic (embedding-based) search via
the `code-index` CLI (https://github.com/janih/code-index-cli), instead
of grep, for conceptual or "how is X done" questions.

## Discover configured projects

Projects available on this machine are listed in
`~/.config/code-index/projects.json` — a JSON array of absolute paths,
keyed by directory basename (e.g. `/Users/x/projektit/veikkaus/spm` →
project name `spm`). Read that file to map a project name to its path
before searching. Absent or empty means no projects are pre-registered;
ask the user for the project's absolute path instead.

Check `code-index status -w <path>` before relying on results for an
unfamiliar project — it shows the configured provider and whether an
index exists (`Collection: ✅ Exists`, `Has Data: ✅ Yes`).

## Run a search

```bash
code-index search -w <path> -q "<query>" --format json -n <limit>
```

- `-w`: the project's absolute path from `projects.json`. Required —
  never omit it; it silently defaults to the current directory's own
  index rather than erroring, which produces confidently wrong ("0
  results") answers for the intended project.
- `-q`: natural-language query.
- `--format json`: machine-parseable output (the default is a
  formatted-text summary; prefer JSON here).
- `-n <limit>`: cap result count (server default is 50; pass 5-10 for a
  focused answer).
- `--directory <prefix>`: optional, restricts results to a sub-directory
  within the project.

If `code-index` is not on PATH, try `~/.cargo/bin/code-index`.

## Parse the JSON output

Each result has this shape:

```json
{
  "id": "...",
  "score": 0.455,
  "payload": {
    "filePath": "/abs/path/to/file.ts",
    "codeChunk": "...",
    "startLine": 110,
    "endLine": 136
  }
}
```

`score` is a 0-1 cosine similarity; treat results below roughly 0.3 as
weak matches. Cite results as `filePath:startLine-endLine`.

## Troubleshooting

- `Found 0 results` on an otherwise-indexed project is almost always a
  workspace-path mismatch, not an empty index — see the `-w` note above.
  The Qdrant collection name is derived from the literal `-w` string
  (not a canonicalized path), so `-w .` and `-w /abs/path` to the same
  directory resolve to two different, unrelated collections.
- "Code index is not configured" — check
  `~/.config/code-index/config.json` (global config) and
  `<path>/.code-index.json` (project-local override); the project needs
  at least an embedder and a Qdrant URL from one of the two.
- No results for a project not yet indexed — indexing (`code-index index
  -w <path>`) and keeping it fresh (`code-index watch`) are separate
  concerns from searching; see this repo's
  `examples/multi-project-watch/` for running several projects'
  watchers continuously.
