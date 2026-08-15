//! Per-model run: index the corpus through a per-model symlink, embed the
//! golden queries at threshold −1.0 (raw scores), persist results.
//!
//! Symlink-farm rationale: Qdrant collections are keyed
//! `ws-<sha256(workspacePath)>`. Each model indexes "its own workspace"
//! (`bench/ws-<slug>` → repo root), which yields one collection per model
//! with per-model `filePath` prefixes — collision-proof even for equal
//! dimensions. Paths are re-based to repo-relative before persisting.

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Instant;

use anyhow::Context;

use code_index::cache::HashCacheManager;
use code_index::embedders::OpenAiCompatibleEmbedder;
use code_index::processors::parser::LineCodeParser;
use code_index::processors::scanner::{DirectoryScanner, ScanCallbacks};
use code_index::shared::constants::MAX_FILE_SIZE_BYTES;
use code_index::traits::{Embedder, VectorStore};
use code_index::vector_store::QdrantVectorStore;

use crate::config::ModelSpec;
use crate::golden::GoldenSet;
use crate::server::ServerMeta;
use crate::types::{Hit, IndexStats, ModelResults, QueryResult};

/// Raw-score search: keep everything cosine can produce.
const NO_THRESHOLD: f32 = -1.0;
const RESULT_LIMIT: u32 = 50;

pub struct RunPaths {
    pub repo_root: PathBuf,
    pub bench_dir: PathBuf,
}

impl RunPaths {
    fn workspace(&self, slug: &str) -> PathBuf {
        self.bench_dir.join(format!("ws-{slug}"))
    }
    fn tmp_dir(&self, slug: &str) -> PathBuf {
        self.bench_dir.join("tmp").join(slug)
    }
    fn results_file(&self, slug: &str) -> PathBuf {
        self.bench_dir.join("results").join(format!("{slug}.json"))
    }
}

pub async fn run_model(
    paths: &RunPaths,
    spec: &ModelSpec,
    meta: &ServerMeta,
    port: u16,
    golden: &GoldenSet,
    qdrant_url: &str,
) -> anyhow::Result<ModelResults> {
    let slug = spec.slug();
    let ws = paths.workspace(&slug);
    setup_workspace(paths, &ws)?;
    let dim = spec.dimension.unwrap_or(meta.dim);

    // Fresh Qdrant state per run: same symlink path -> same collection name,
    // so a stale collection (different model at same slug) must not survive.
    let store = Arc::new(QdrantVectorStore::new(
        &ws.to_string_lossy(),
        Some(qdrant_url),
        dim,
        None,
    ));
    store.delete_collection().await.ok();
    store.initialize().await?;

    // Fresh cache dir: cache hit + deleted collection would skip files.
    let cache_dir = paths.tmp_dir(&slug);
    if cache_dir.exists() {
        std::fs::remove_dir_all(&cache_dir)
            .with_context(|| format!("cannot clean {}", cache_dir.display()))?;
    }
    std::fs::create_dir_all(&cache_dir)?;
    let cache = Arc::new(HashCacheManager::new(&ws, Some(cache_dir)));

    let mut ignore_builder = ignore::gitignore::GitignoreBuilder::new(&ws);
    if ws.join(".gitignore").exists() {
        ignore_builder.add(ws.join(".gitignore"));
    }
    // Keep benchmark artifacts out of the corpus so golden queries about the
    // product cannot be "answered" by harness files.
    ignore_builder.add_line(None, "bench/")?;
    ignore_builder.add_line(None, "/.code-index.json")?;
    let matcher = ignore_builder.build()?;

    let base_url = format!("http://127.0.0.1:{port}/v1");
    let embedder = Arc::new(OpenAiCompatibleEmbedder::new(
        base_url,
        "bench".to_string(),
        Some(meta.model_id.clone()),
    ));

    // ---- Index ----
    let scanner = DirectoryScanner::new(
        embedder.clone(),
        store.clone(),
        Arc::new(LineCodeParser::new()),
        cache,
        matcher,
        None,
        Some(MAX_FILE_SIZE_BYTES),
    );
    let started = Instant::now();
    let outcome = scanner
        .scan_directory(&ws, ScanCallbacks::default(), None)
        .await
        .context("indexing failed")?;
    let index_secs = started.elapsed().as_secs_f64();
    println!(
        "[{slug}] indexed {} files, {} blocks in {index_secs:.1}s",
        outcome.stats.processed, outcome.total_block_count
    );

    // ---- Query ----
    // Warm-up (excluded from latency): first call pays tokenizer/init costs.
    embedder
        .create_embeddings(&["warm up".to_string()], None, true)
        .await?;

    let mut queries = Vec::with_capacity(golden.queries.len());
    for q in &golden.queries {
        let text = match &spec.query_prefix {
            Some(prefix) => format!("{prefix}{}", q.query),
            None => q.query.clone(),
        };
        let started = Instant::now();
        let response = embedder
            .create_embeddings(&[text], None, true)
            .await
            .context("query embedding failed")?;
        let vector = response
            .embeddings
            .into_iter()
            .next()
            .context("empty query embedding")?;
        let found = store
            .search(&vector, None, Some(NO_THRESHOLD), Some(RESULT_LIMIT))
            .await
            .context("query failed")?;
        let latency_ms = started.elapsed().as_secs_f64() * 1000.0;

        let hits: Vec<Hit> = found
            .iter()
            .enumerate()
            .map(|(i, r)| {
                let raw_path = r
                    .payload
                    .as_ref()
                    .map(|p| p.file_path.as_str())
                    .unwrap_or_default();
                let ws_str = ws.to_string_lossy();
                let rel = raw_path
                    .strip_prefix(ws_str.as_ref())
                    .map(|s| s.trim_start_matches('/').to_string())
                    .unwrap_or_else(|| raw_path.to_string());
                Hit {
                    rank: i + 1,
                    path: rel,
                    score: r.score,
                    start_line: r.payload.as_ref().map(|p| p.start_line as u64).unwrap_or(0),
                    end_line: r.payload.as_ref().map(|p| p.end_line as u64).unwrap_or(0),
                }
            })
            .collect();
        queries.push(QueryResult {
            qid: q.id.clone(),
            latency_ms,
            hits,
        });
    }

    let results = ModelResults {
        name: spec.name.clone(),
        slug: slug.clone(),
        gguf: spec.gguf.clone(),
        dimension: dim,
        n_ctx: meta.n_ctx,
        query_prefix: spec.query_prefix.clone(),
        index: IndexStats {
            files_processed: outcome.stats.processed,
            files_skipped: outcome.stats.skipped,
            blocks: outcome.total_block_count,
            index_secs,
        },
        queries,
    };

    let file = paths.results_file(&slug);
    if let Some(parent) = file.parent() {
        std::fs::create_dir_all(parent)?;
    }
    std::fs::write(&file, serde_json::to_vec_pretty(&results)?)
        .with_context(|| format!("cannot write {}", file.display()))?;
    println!("[{slug}] results written to {}", file.display());
    Ok(results)
}

/// (Re)creates the `bench/ws-<slug>` symlink pointing at the repo root.
fn setup_workspace(paths: &RunPaths, ws: &Path) -> anyhow::Result<()> {
    let target = paths
        .repo_root
        .canonicalize()
        .context("cannot canonicalize repo root")?;
    if ws.symlink_metadata().is_ok() {
        std::fs::remove_file(ws).context("cannot remove stale workspace symlink")?;
    }
    std::os::unix::fs::symlink(&target, ws)
        .with_context(|| format!("symlink {} -> {}", ws.display(), target.display()))?;
    Ok(())
}

/// Deletes the model's Qdrant collection, cache dir and symlink.
pub async fn cleanup_model(paths: &RunPaths, spec: &ModelSpec, qdrant_url: &str) {
    let slug = spec.slug();
    let ws = paths.workspace(&slug);
    let store = QdrantVectorStore::new(&ws.to_string_lossy(), Some(qdrant_url), 1, None);
    store.delete_collection().await.ok();
    if let Err(err) = std::fs::remove_dir_all(paths.tmp_dir(&slug)) {
        if err.kind() != std::io::ErrorKind::NotFound {
            eprintln!("[{slug}] cache cleanup failed: {err}");
        }
    }
    let _ = std::fs::remove_file(&ws);
}
