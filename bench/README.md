# Search-quality benchmark

Compares local embedding models on **codebase search quality** over this
repository, using the real library pipeline (parser → scanner → embedder →
Qdrant) — the same code `code-index index/search` runs.

## Usage

```sh
# Qdrant must be running on :6333
 cargo run --release --example search_quality -- run                     # all corpora × all models
 cargo run --release --example search_quality -- run --corpus ts-reference   # one corpus
 cargo run --release --example search_quality -- run --model Qwen3-Embedding-0.6B-Q8
 cargo run --release --example search_quality -- analyze               # re-report from saved results
 cargo run --release --example search_quality -- clean                 # drop collections/caches/symlinks
```

`run` iterates the configured **corpora** (`bench/models.json`): for each
corpus × model it starts llama-server (sequential, port 8099), indexes the
corpus through a per-model symlink (`bench/ws-<corpus>-<model>` → corpus
root, giving each pair its own Qdrant collection even at equal
dimensions), embeds the golden queries at threshold −1.0 with limit 50,
writes `bench/results/<corpus>/<model>.json`, cleans up (`--keep` to
retain collections), and finally analyzes every corpus. Results persist,
so `analyze` re-computes metrics without touching any server.

### Corpora

| Corpus | What | Why |
|---|---|---|
| `code-index-cli` | this repo (Rust) | the shipped use case |
| `ts-reference` | `first-version-build` via `git worktree` (TypeScript, bootstrapped automatically) | cross-language generalization check |

Corpus config: `name`, `root` (relative), `exclude` (gitignore patterns
on top of the corpus's own `.gitignore`; the self corpus excludes `bench/`
so golden files cannot answer their own queries), `worktreeBranch`
(auto-creates the root as a git worktree).

## Layout

| Path | Committed | Contents |
|---|---|---|
| `bench/models.json` | yes | llama-server path, corpora + model registry (GGUF, `modelId`, optional `queryPrefix`, `baseline`) |
| `bench/golden/<corpus>.jsonl` | yes | labeled query sets — the core asset |
| `bench/results/`, `bench/reports/`, `bench/tmp/`, `bench/ws-*` | no | run artifacts (gitignored) |

## Golden set conventions

One JSON object per line:

```json
{"id":"q01","query":"how do I configure a custom ollama server URL",
 "category":"config","relevant":{"src/config/loader.rs":2}}
```

- **File-level, graded**: 2 = the file that answers the query, 1 = partial
  / supporting (docs count when they genuinely answer). Paths are
  repo-relative.
- Queries are phrased as real users would search; avoid pasting identifier
  names into behavior queries (that changes the task to exact-match).
- Categories: usage, symbol, behavior, config, concurrency, cache, qdrant,
  search.
- Optional `"split": "dev"` marks threshold-tuning queries (default
  `eval`): headline metrics report the eval split only; the threshold
  sweep selects on dev and evaluates the winner on eval.
- Optional `"directory"` marks `--directory`-slice queries: searched with
  a server-side prefix filter + client post-filter, exactly the product
  path; reported in a dedicated section.

## Metrics

- **Recall@1/5/10, MRR@10, nDCG@10** — file level (hits deduped by best
  rank), the primary cross-model numbers.
- **ROC-AUC / score gap** — threshold-independent discrimination from raw
  cosine scores (raw scores are *not* comparable across models).
- **Threshold sweep** — macro-F1 at 0.05–0.95; reports the per-model
  threshold that would replace the global 0.4 guess.
- **Latency** p50/p95 (embed + search, warm-up excluded) and index time.
- **Paired bootstrap** Δrecall@10 vs the baseline (95% CI, 1000 resamples).

## Notes / limitations

- M1 runs embed **raw queries** for every model (no instruction prefixes),
  matching how the shipped CLI searched at the time; M2 introduced the
  prefix variants, M3 moved the winners into the product (see above).
- Dimensions are auto-detected from the server's `/v1/models` metadata
  (`meta.n_embd`); set `"dimension"` explicitly if a build omits it.
- Block counts must match across models in the report's sanity table —
  chunking is deterministic, so any mismatch means a broken run.
- **Query-prefix ablation**: entries with `queryPrefix` in `models.json`
  (e.g. `+task`, `+instruct`, `+q` variants) prepend the model's
  instruction template to queries only — documents are always embedded
  raw, matching the shipped CLI. That asymmetry is deliberate: it measures
  what a query-side-only change would buy the product. NOTE: since the
  product gained benchmark-derived profiles (M3), base entries with a
  canonical `modelId` (e.g. `unsloth_embeddinggemma-300M-Q8_0`,
  `Qwen3-Embedding-0.6B`) already apply the product prefix — so base and
  `+`-variant converge to identical results, which doubles as a wiring
  consistency check. Raw-query ablation requires a model without a
  product profile or a non-canonical `modelId`.
- **llama-server flags**: the harness starts servers with
  `--batch-size 8192 --ubatch-size 4096`. The 512 default silently drops
  whole batches when a corpus has very long single lines
  (package-lock.json) for some tokenizers — the first ts-reference run
  lost 240 of 515 blocks that way before the sanity check caught it.
- Model-specific contexts (e.g. `n_ctx_train: 2048` for embeddinggemma)
  are recorded in results; long blocks can be truncated by the server.
