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

## Safety Rules

- Never store secrets in git. API keys come from config files / env vars only.
- Never commit files the user did not ask you to commit, and never discard
  work you did not create yourself (`git reset --hard` = ask first).
- Behavioral deviations from the TS version must be documented in
  `REWRITE-PLAN.md`.
- `unwrap()` is for tests and provably-safe paths (e.g. lock recovery);
  production paths return `anyhow::Result`.

## Porting Conventions

- Port faithfully; where the TS version has a live bug (e.g. scanner
  delete-after-upsert), fix it and record the deviation in REWRITE-PLAN.md
  with evidence.
- Constants must keep TS values (`src/shared/constants.rs`, compile-time
  asserted); cross-language determinism (uuid-v5 point ids, namespace
  `f47ac10b-58cc-4372-a567-0e02b2c3d479`) is locked by tests.
- Tests: no network — pure functions + trait mocks; the live E2E is manual
  (needs Qdrant + an embedding endpoint).
