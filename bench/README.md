# Search-quality benchmark

Compares local embedding models on **codebase search quality** over this
repository, using the real library pipeline (parser → scanner → embedder →
Qdrant) — the same code `code-index index/search` runs.

## Usage

```sh
# Qdrant must be running on :6333
cargo run --release --example search_quality -- run            # all models
cargo run --release --example search_quality -- run --model Qwen3-Embedding-0.6B-Q8
cargo run --release --example search_quality -- analyze        # re-report from saved results
cargo run --release --example search_quality -- clean          # drop collections/caches/symlinks
```

`run` starts llama-server per model from `bench/models.json` (sequential,
port 8099), indexes the repo through a per-model symlink
(`bench/ws-<slug>` → repo root, which gives each model its own Qdrant
collection even at equal dimensions), embeds the golden queries at
threshold −1.0 with limit 50, writes `bench/results/<slug>.json`, cleans
up (`--keep` to retain collections), and prints the report. Results files
persist, so `analyze` re-computes metrics without touching any server.

## Layout

| Path | Committed | Contents |
|---|---|---|
| `bench/models.json` | yes | llama-server path + model registry (GGUF, optional `queryPrefix`, `baseline`) |
| `bench/golden/*.jsonl` | yes | the labeled query set — the core asset |
| `bench/results/`, `bench/reports/`, `bench/tmp/`, `bench/ws-*` | no | run artifacts (gitignored) |

## Golden set conventions

One JSON object per line:

```json
{"id":"q01","query":"how do I configure a custom ollama server URL",
 "category":"config","relevant":{"src/config/loader.rs":2}}
```

- **File-level, graded**: 2 = the file that answers the query, 1 = partial
  / supporting. Paths are repo-relative.
- Queries are phrased as real users would search; avoid pasting identifier
  names into behavior queries (that changes the task to exact-match).
- Categories: usage, symbol, behavior, config, concurrency, cache, qdrant,
  search.
- ~6 queries of the set are a dev split for threshold tuning — do not
  report tuned numbers on the full set.

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
  matching how the shipped CLI searches today. `queryPrefix` in
  `models.json` enables per-model instruction ablation later.
- Dimensions are auto-detected from the server's `/v1/models` metadata
  (`meta.n_embd`); set `"dimension"` explicitly if a build omits it.
- Block counts must match across models in the report's sanity table —
  chunking is deterministic, so any mismatch means a broken run.
- Model-specific contexts (e.g. `n_ctx_train: 2048` for embeddinggemma)
  are recorded in results; long blocks can be truncated by the server.
