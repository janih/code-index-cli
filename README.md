# code-index-cli (Rust)

Standalone CLI that indexes a codebase into a local **Qdrant** vector store and
answers semantic search queries, as a single small binary (≈4 MB, cold start in
milliseconds — no Node.js or any runtime required).

This is a Rust rewrite of the original Node.js version (kept on the
`first-version-build` branch). See `REWRITE-PLAN.md` for the porting status,
parities and documented deviations.

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

openai, ollama, openai-compatible, gemini, mistral, vercel-ai-gateway,
openrouter. **Bedrock is not ported yet** (deferred, needs AWS SigV4).
Unknown models need `embedder.modelDimension` in the config.

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
