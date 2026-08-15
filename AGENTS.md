# AGENTS.md — code-index-cli (Rust) Development Guide

## Project Overview

**code-index-cli** (`code-index`) is a standalone Rust CLI that indexes
codebases into a **Qdrant** vector store and answers semantic search queries
via embedding providers (OpenAI, Ollama, OpenAI-compatible, Gemini, Mistral,
Vercel AI Gateway, OpenRouter). It descends from an earlier Node.js tool
(preserved on the `first-version-build` branch for history and as a
benchmark corpus); this codebase is the canonical implementation.

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
  the primary extension seams of the CLI.
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
  (see `src/config/loader.rs`), validated in `schema.rs` (range checks
  beyond serde types).
- `src/shared/` — constants, supported extensions, model profiles,
  gitignore matcher helper, cancellation token, logger (`src/log.rs`).
- `bench/` + `examples/search_quality/` — embedding-model search-quality
  benchmark (golden-set eval, llama-server rotation); see `bench/README.md`.
  Machine-specific paths live in gitignored `bench/models.local.json`.

## Safety Rules

- Never store secrets in git. API keys come from config files / env vars only.
- Never commit files the user did not ask you to commit, and never discard
  work you did not create yourself (`git reset --hard` = ask first).
- `unwrap()` is for tests and provably-safe paths (e.g. lock recovery);
  production paths return `anyhow::Result`.

## Conventions

- Index-format constants (block limits, batch thresholds, the Qdrant UUID
  namespace `f47ac10b-58cc-4372-a567-0e02b2c3d479`) are compile-time asserted
  in `src/shared/constants.rs`: they define the stored index format, and
  changing them orphans existing indexes. Deterministic uuid-v5 point ids
  are locked by tests.
- Tests: no network — pure functions + trait mocks; the live E2E is manual
  (needs Qdrant + an embedding endpoint).

## Known limitations & notes

- **Bedrock** is not implemented — the factory fails with an explicit error
  (needs AWS SigV4 signing).
- **`--dry-run`** is accepted for flag compatibility but not implemented;
  it logs a warning.
- **`indexing.includeExtensions`** is accepted but inert; the scanner uses
  the built-in extension list (`src/shared/supported_extensions.rs`).
- **Idle deletions** — files deleted while the CLI is not watching are not
  purged from the index; `clear` + fresh `index` resets.
- **Lost cache file** — if the hash cache is lost while the Qdrant
  collection keeps data, re-indexed files' old points are not removed (the
  scanner only deletes stale points for files it has cached); the watcher
  deletes unconditionally on touch; `clear && index` recovers.
