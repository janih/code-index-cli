# code-index-cli (Rust)

Standalone CLI that indexes a codebase into a local **Qdrant** vector store and
answers semantic search queries, as a single small binary (≈4 MB, cold start in
milliseconds — no Node.js or any runtime required).

This is a Rust rewrite of the original Node.js version (kept on the
`first-version-build` branch). The port is complete (v0.2.x); intentional
behavioral differences from the original are documented in the
**Deviations** section of `AGENTS.md`.

## Requirements

- A running **Qdrant** (e.g. `docker run -p 6333:6333 qdrant/qdrant`)
- An embedding endpoint: OpenAI API key, Ollama, or any **OpenAI-compatible**
  server (llama-server, vLLM, LiteLLM, LM Studio, …)
- Rust toolchain only if building from source (`cargo build --release`)

## Quickstart (local llama-server example)

```sh
code-index init                       # writes .code-index.json
cat > .code-index.json <<EOF
{
  "embedder": {
    "provider": "openai-compatible",
    "compatibleBaseUrl": "http://localhost:8089/v1",
    "compatibleApiKey": "test",
    "modelId": "embeddinggemma-300M",
    "modelDimension": 768
  },
  "qdrant": { "url": "http://localhost:6333" }
}
EOF
code-index index                      # scan + embed + upsert
code-index search -q "how are CLI arguments parsed?"
code-index watch                      # index once, then live-reindex on changes
```

## Commands

| Command         | Purpose                                              |
| --------------- | ---------------------------------------------------- |
| `init`          | Create a template config (`--force` to overwrite)    |
| `index`         | Scan workspace and index code blocks                 |
| `search`        | Semantic search (`-q/--query`, `--format json`)       |
| `watch`         | Index, then keep the index fresh on file changes     |
| `status`        | Show config + Qdrant connectivity and cache stats    |
| `clear`         | Delete the workspace's Qdrant collection and cache   |

`--debug` on any command enables internal `[DEBUG]` logs.

## Configuration layering

later layers override earlier ones:

**defaults** → `~/.config/code-index/config.json` → `./.code-index.json` →
env vars (`CODE_INDEX_CLI_*`, e.g. `CODE_INDEX_CLI_EMBEDDER_API_KEY`) → CLI flags
(`--provider`, `--model`, `--api-key`, `--qdrant-url`, …).

## Providers

**Primary use case: a local setup** — Qdrant on localhost (Docker) + a
local embedding model served over an **OpenAI-compatible** endpoint
(e.g. llama-server). That combination is end-to-end tested and used
by the maintainer day to day.

Provider support matrix:

| Provider | Status |
| --- | --- |
| openai-compatible | E2E-tested (llama-server + local Qdrant) |
| openai | ported, unit-tested |
| ollama, gemini, mistral, vercel-ai-gateway, openrouter | ported but **not live-tested** (no API keys at hand) |
| bedrock | **not ported** — the factory fails with an explicit "deferred" error (needs AWS SigV4 signing) |

Unknown models need `embedder.modelDimension` in the config.

**llama-server tip:** start it with `--ubatch-size 4096` (and `--batch-size
8192`). The default physical batch (512 tokens) silently fails whole
embedding batches when the indexed repo contains very long single lines
(e.g. minified JSON/JS or `package-lock.json`) — `code-index` logs
`Error processing batch: … increase the physical batch size` and skips
those files.

## Data locations

- Qdrant collection per workspace: `ws-<sha256(path)[..16]>`
- Hash cache: `~/.cache/code-index/cache-<sha256(path)>.json`

## Development

```sh
cargo fmt
cargo clippy --all-targets -- -D warnings
cargo test
```

See `AGENTS.md` for contributor conventions (atomic commits etc.).
