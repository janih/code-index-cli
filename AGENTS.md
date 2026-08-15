# AGENTS.md — code-index-cli (Rust) Development Guide

## Project Overview

**code-index-cli** (`code-index`) is a standalone Rust CLI that indexes
codebases into a **Qdrant** vector store and answers semantic search queries
via embedding providers (OpenAI, Ollama, OpenAI-compatible, Gemini, Mistral,
Vercel AI Gateway, OpenRouter). This repository hosts the Rust rewrite; the
original Node.js implementation lives on the `first-version-build` branch as
the behavioral reference (`git show first-version-build:src/...`).

## Build, Lint, Test

- **Build**: `cargo build` / `cargo build --release`
- **Lint**: `cargo fmt --check` and `cargo clippy --all-targets -- -D warnings` — both must pass
- **Test**: `cargo test` (unit tests are co-located in `#[cfg(test)]` modules)
- **Run**: `cargo run -- <command>` e.g. `cargo run -- status`
- **Cross-compile**: `cargo build --release --target x86_64-unknown-linux-musl`

Run fmt + clippy + tests **before every commit**.

## Git Workflow

- Branch convention: feature branches off `main`; merges via PR.
- **Atomic commits**: one logical change per commit. Stage files explicitly
  (`git add <file>`), never `git add .`.
- Commit message format: `type: summary` where type is one of
  feat / fix / docs / refactor / test / chore.
- Before committing: confirm build, lint and tests pass.
- Ask before modifying `package.json`/`Cargo.toml` manifest structure,
  lockfiles, or deleting files.

## Codebase Structure

- `src/traits.rs` — Embedder / VectorStore / CodeParser / CacheManager traits;
  the seams the TS test-suite mocked, now real Rust traits.
- `src/cli.rs` — clap derive definitions; the binary entry is `src/main.rs`
  (thin) → `cli::run()` starting a tokio multi-thread runtime.
- `src/commands/mod.rs` — one async handler per command
  (init / index / search / watch / status / clear).
- `src/core/` — orchestrator (scan flow), search service, service factory
  (dependency wiring), state manager (IndexingState + subscribers).
- `src/embedders/` — HTTP embedders over `reqwest`; shared OpenAI-shape
  parsing lives in `openai.rs` (reused by openai-compatible + simple_http).
- `src/vector_store/qdrant.rs` — Qdrant REST client.
- `src/processors/` — scanner (buffer_unordered pipeline), parser
  (line/markdown chunking), watcher (`notify` + 500ms debounce).
- `src/cache/manager.rs` — file-hash cache with debounced disk persistence.
- `src/config/` — layered config: defaults < user < project < env < flags
  (see `src/config/loader.rs`), validated in `schema.rs` (zod parity).
- `src/shared/` — constants, supported extensions, model profiles,
  gitignore matcher helper, cancellation token, logger (`src/log.rs`).
- `bench/` + `examples/search_quality/` — embedding-model search-quality
  benchmark (golden-set eval, llama-server rotation); see `bench/README.md`.
  Machine-specific paths live in gitignored `bench/models.local.json`.

## Safety Rules

- Never store secrets in git. API keys come from config files / env vars only.
- Never commit files the user did not ask you to commit, and never discard
  work you did not create yourself (`git reset --hard` = ask first).
- Behavioral deviations from the TS version must be recorded in the
  **Deviations** section below.
- `unwrap()` is for tests and provably-safe paths (e.g. lock recovery);
  production paths return `anyhow::Result`.

## Porting Conventions

- Port faithfully; where the TS version has a live bug (e.g. scanner
  delete-after-upsert), fix it and record the deviation below
  with evidence.
- Constants must keep TS values (`src/shared/constants.rs`, compile-time
  asserted); cross-language determinism (uuid-v5 point ids, namespace
  `f47ac10b-58cc-4372-a567-0e02b2c3d479`) is locked by tests.
- Tests: no network — pure functions + trait mocks; the live E2E is manual
  (needs Qdrant + an embedding endpoint).

## Deviations from the TS reference

Where the Rust port intentionally differs from `first-version-build`.
Keep this list current when behavior diverges further.

- **Scanner delete-before-upsert** — stale points are deleted BEFORE
  upserting a file's new blocks; TS deleted after `Promise.all`, wiping
  the fresh points.
- **Parser start-line fix** — fallback chunking set the next block's start
  line one too high (ported TS bug). Fixing it changes segment hashes →
  uuid-v5 point ids for multi-block files: after upgrading from a
  pre-v0.2.1 index, run `clear` + `index` once per workspace.
- **Embedding count mismatch aborts** — a provider returning fewer
  embeddings than input texts fails the batch explicitly; the TS zip
  equivalent would silently truncate.
- **No DELETE storm on first index** — the scanner skips the stale-point
  DELETE for files without a cached hash (never indexed ⇒ nothing stale).
  Caveat: a lost cache file beside a live collection can leave stale
  points for re-indexed files (`clear && index` recovers); the watcher
  still deletes unconditionally.
- **`--directory` resolution** — verified against live Qdrant (the second
  payload-index creation makes `filePath` a text index; `match: {text}`
  AND-token filters prefixes). Relative prefixes resolve against the
  workspace string from the CLI, not `current_dir()` — TS resolves via
  `process.cwd()`, which canonicalizes symlinks on macOS and matched
  nothing.
- **`indexing.maxFileSizeBytes` honored** — parsed-but-unused in TS (always
  1 MiB constant). `indexing.includeExtensions` stays accepted-but-inert
  in both versions (wiring it means teaching the parser custom
  extensions).
- **Benchmark-derived model profiles** — embeddinggemma-300M and
  Qwen3-Embedding-0.6B carry query prefixes + per-model score thresholds
  (0.55) in the openai-compatible profile table, sourced from
  `bench/` (search-quality benchmark). TS ships no such profiles for
  these models; the mechanism (query prefix + threshold) is pre-existing
  TS behavior via nomic-embed-code.
- **API-key redaction wired** — provider/network error paths run through
  `sanitize_error_message`; TS ports the helper but never wires it.
- **Bytes vs UTF-16 code units** — chunk sizing (`processors/parser.rs`)
  and token estimation (`embedders/openai.rs::estimate_tokens`) count
  bytes where JS counted UTF-16 units; boundary-level differences for
  non-ASCII content only.
- **`--dry-run`** — accepted for flag compatibility but not implemented
  (same as TS); logs a warning instead of pretending.
- **Bedrock deferred** — the factory fails with an explicit error
  (needs AWS SigV4 signing); not ported.
- **Known parity gap (both versions)** — files deleted while the CLI is
  idle are not purged from the index on next startup; `clear` + fresh
  `index` resets.
