# Rust Rewrite Plan — code-index-cli

Rewrite of the TypeScript/Node.js code-index-cli as a standalone Rust binary.

- **Reference implementation:** branch `first-version-build` (read-only reference, e.g. `git show first-version-build:src/index.ts`)
- **Work branch:** `rust-rewrite` (branched from `main`)
- **Merge target:** `main` once parity is reached

## Goals

- Single static binary (musl for Linux, universal for macOS, MSVC for Windows); no runtime required
- Fast startup (<10 ms) — matters for repeated `search` invocations
- CLI flag-compatible with the TS version (commands: `init`, `index`, `search`, `watch`, `status`, `clear`)
- Config-compatible: same `.code-index.json` format, same layering (defaults → user → project → env → CLI flags), same `CODE_INDEX_CLI_*` env vars
- All 8 embedder providers: openai, ollama, openai-compatible, gemini, mistral, vercel-ai-gateway, bedrock (cargo feature), openrouter
- Behavioral parity: retries/backoff, API-key redaction in errors, per-model dimensions/score thresholds, cache semantics

## Non-goals

- No indexing-throughput claims: workload is network-bound (embedding APIs, Qdrant HTTP); Rust does not make it faster
- No new UX/features during the port, except where Rust makes them nearly free (native tree-sitter chunking is a post-parity bonus, not a parity item)
- Not keeping the Node build working on this branch

## TS → Rust module / crate mapping

| TS (first-version-build) | Rust module | Crate(s) | Notes |
|---|---|---|---|
| `index.ts` (commander) | `cli.rs` | `clap` derive | 6 commands, same flags |
| `config/` (zod + custom layering) | `config/` | `serde`, `serde_json`, `figment` or hand-rolled merge | Keep precedence order and ENV_MAP identical |
| `embedders/*` (openai SDK + fetch) | `embedders/*` | `reqwest` (rustls), `serde_json` | One shared `HttpEmbedderBase`: retry+sanitize |
| `embedders/bedrock.ts` | `embedders/bedrock.rs` | `aws-sdk-bedrockruntime` | cargo feature `bedrock` (compile time/binary size) |
| `vector-store/qdrant-client.ts` | `vector_store/qdrant.rs` | `reqwest` | Thin REST wrapper, same as TS |
| `processors/parser.ts` (line-based) | `processors/parser.rs` | — | Pure string logic, trivial port |
| `processors/scanner.ts` (p-limit) | `processors/scanner.rs` | `ignore` (ripgrep engine), `tokio::sync::Semaphore` | gitignore-aware walk comes free |
| `processors/file-watcher.ts` (chokidar) | `processors/watcher.rs` | `notify` + tokio channels | Phase 3 |
| `cache/cache-manager.ts` | `cache/manager.rs` | `serde_json`, `sha2` | Debounced flush via tokio |
| `core/orchestrator.ts` | `core/orchestrator.rs` | `tokio`, `indicatif` | |
| `core/state-manager.ts` (EventEmitter) | `core/state.rs` | enum + `tokio::sync::watch` | No EventEmitter needed |
| `utils/logger.ts` | `log.rs` | plain macros + `std::sync::atomic` | Keep `[INFO]/[WARN]/[ERROR]/[DEBUG]` format, `--debug` gate |
| `shared/*` | `shared/*` | — | Constants + model profiles port 1:1 |
| `interfaces/*` | `traits.rs` (+ per-module) | — | `Embedder`, `VectorStore`, `CodeParser`, `CacheManager` as async traits |
| vitest suite (277 tests) | `#[cfg(test)]` + `cargo test` | `mockall` or hand-rolled fakes | Port ~1:1 |

## Phase plan

Each phase ends in a working, committed state. Commits are atomic per AGENTS.md workflow rules.

### Phase 0 — Scaffold ✅ target: binary runs `--help`

- [x] Branch `rust-rewrite` from `main`, this plan committed
- [x] `cargo` binary crate `code-index`, edition 2021
- [x] `clap` derive CLI matching all 6 commands + flags exactly (from `src/index.ts`)
- [x] Module skeleton (empty `mod` files with purpose doc comments)
- [x] `cargo build`, `cargo clippy`, `cargo test` green
- [x] `.gitignore` for `target/`

**Acceptance:** `code-index --help` lists init/index/search/watch/status/clear with TS-equivalent flags. ✅ (635 KB release binary)

### Phase 1 — Vertical slice: `index` + `search` work for OpenAI ✅

Status (done): layered config, traits (Embedder/VectorStore/CodeParser/
CacheManager), Qdrant REST store, OpenAI embedder (batching+retry), line/
markdown parser, scanner pipeline (buffer_unordered + JoinSet + Semaphore),
cache with debounced persistence, StateManager, ServiceFactory,
Orchestrator, SearchService — all six commands wired; watch/init-prompts
deferred as planned. 118 unit tests green, release binary ~4.2 MB.
Pending within this phase: live E2E against real Qdrant + OpenAI key
(needs those services; can also be verified via Ollama after Phase 2
brings that provider in).

Deviations found during the port (agreed as TS-gap mirrors):
- (review round 1, v0.2.1) Parser fallback-chunk start lines were +1 due
  to a ported TS bug (`i + 2` after pre-push flush) — FIXED in the port;
  changes point IDs for multi-block files -> clear + reindex once.
- (review round 1) The `zip(blocks, embeddings)` truncation concern and
  per-file DELETE storms are scheduled alongside sanitize-wiring; see the
  tracking list in CODE-REVIEW.md.

- `--dry-run` is accepted but was never wired into the TS pipeline;
  the Rust port logs a warning instead of pretending it works.
- Chunk sizing counts bytes where JS counted UTF-16 code units
  (equal for ASCII; slight boundary differences for non-ASCII).


Riskiest integration first: real embedder → real Qdrant round trip.

- [ ] `shared/constants.rs`, `shared/embedding_models.rs`, `shared/supported_extensions.rs` (+ tests, 1:1 data port)
- [ ] `shared/supported_extensions.rs` (+ tests)
- [ ] `log.rs` with `--debug` gating + `sanitize_error_message` (redact `sk-*`, Bearer, `api_key=`)
- [ ] `config/`: schema (serde), layered loader, manager (+ tests)
- [ ] `traits.rs`: `Embedder`, `VectorStore`, `CodeParser`, `CacheManager`
- [ ] `embedders/openai.rs`: embeddings API, batching (MAX_BATCH_TOKENS), retry/backoff (MAX_BATCH_RETRIES=3, INITIAL_RETRY_DELAY_MS=500), validation
- [ ] `vector_store/qdrant.rs`: initialize/upsert/search/delete-by-path/clear (+ mocked tests)
- [ ] `cache/manager.rs`: file-hash cache, debounced flush (+ tests)
- [ ] `processors/parser.rs`: line-chunking + markdown sections (+ tests)
- [ ] `processors/scanner.rs`: walk via `ignore`, concurrency limits (PARSING_CONCURRENCY=10, MAX_PENDING_BATCHES=20)
- [ ] `core/orchestrator.rs`, `core/search_service.rs`, `core/state.rs`, `core/service_factory.rs`
- [ ] `commands/`: `init`, `index`, `search` wired end-to-end

**Acceptance:** against a local Qdrant (`docker run -p 6333:6333 qdrant/qdrant`), `code-index index` then `code-index search -q ...` returns results for a sample repo with an OpenAI-compatible endpoint.

### Phase 2 — Remaining providers + `status`/`clear` ✅

Status (done): all non-AWS providers ported (ollama, openai-compatible,
gemini, mistral, vercel-ai-gateway, openrouter); status/clear live.
**Bedrock deferred** (needs hand-rolled SigV4 or aws-sdk — user
deprioritized). Live-tested providers can only be E2E'd with real keys;
openai-compatible was E2E'd against llama-server + local Qdrant,
including the scanner ordering fix (delete-before-upsert wipe bug —
deviation from a latent TS bug).

- [ ] ollama, gemini, mistral, openai-compatible, vercel-ai-gateway, openrouter (~shared base each)
- [ ] `bedrock` behind `--features bedrock`
- [ ] `commands/status.rs`, `commands/clear.rs`
- [ ] Port provider tests (mocked HTTP)

**Acceptance:** provider matrix tests pass; `status`/`clear` behave like TS.

### Phase 3 — `watch` ✅

Status (done): notify-based watcher, 500ms debounced batches, macOS
FSEvents fixed (path rebasing + gone-file promote-to-delete), graceful
Ctrl+C shutdown; verified live. Known parity gap (same in TS): files
deleted while the CLI is idle are not purged from the index on next
startup — use `clear` + fresh `index` to reset.

- [ ] `processors/watcher.rs` on `notify`, debounce, incremental reindex, deletes
- [ ] `commands/watch.rs`, Ctrl-C graceful stop (state: Stopping)

**Acceptance:** editing/adding/deleting a file updates the index; clean shutdown.

### Phase 4 — Parity, tests, releases

- [x] Port remaining unit tests (aim ≥ TS count where meaningful, ~277)
- [~] CLI snapshot parity: `--help` format differs (clap vs commander) — same commands/flags, verified manually
- [x] GitHub Actions: fmt + clippy + tests + 5-target release-binary matrix (ci.yml)
- [x] Update README + AGENTS.md for Rust; remove REWRITE-PLAN.md last
- [ ] Merge to `main`; tag `v0.2.0`

**Stretch (post-parity):** real tree-sitter chunking (`tree-sitter` + grammars compile natively — the TS version ships `web-tree-sitter` but never uses it).

## Behavioral parity checklist (verified)

- [x] Config precedence: defaults < user < project < env < CLI flags
  (loader tests cover layering + string coercion)
- [x] Unknown model → `embedder.modelDimension` fallback
  (factory tests: fallback used, error when absent)
- [~] Error messages redact API keys — helper ported
  (`shared/validation.rs`) BUT: the TS version never wires it into any
  command/embedder either (only own tests). Honest parity = unused in
  both. Improvement candidate, not a deviation.
- [x] Block limits: 50/1000/1.15/200 (const-asserted; parser tests)
- [x] Search defaults: limit 50 (schema bounds 10..200), min score 0.4
  (schema-validated, load fails on violation — zod parity)
- [x] Qdrant UUID namespace + deterministic v5 point IDs
  (locked against RFC-4122 cross-implementation values)
- [x] 1MB file skip, 50k listing cap, batch threshold 60
- [x] Live E2E (manual): llama-server embeddinggemma-300M +
  Docker Qdrant — index/search/incremental/watch lifecycle verified
- [~] Test count ~146 vs TS ~277: TS suites mock HTTPS/SDK responses for
  8 embedders + watcher event plumbing; the port covers the same
  behaviors via unit tests on pure functions + trait mocks (the seams
  are shared). Network-level mocks are deferred to the integration
  stage if desired.

## Testing strategy

- Unit tests co-located (`#[cfg(test)] mod tests`), mirroring TS test intent
- HTTP mocked via injectable base URL / trait fakes (no network in tests)
- Parser, config, constants, model profiles are pure — direct value tests
- One manual end-to-end checklist against local Qdrant + Ollama in Phase 1, scripted in Phase 4 CI (service container if feasible)

## Risks

| Risk | Mitigation |
|---|---|
| Bedrock SigV4 friction + build bloat | optional cargo feature, port last |
| Parity drift in retry/sanitization details | port TS tests literally; this checklist |
| Rewrite stalls mid-way | phase = working binary; vertical slice in Phase 1 de-risks early |
