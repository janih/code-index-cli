//! Per-model run: index the corpus through a per-model symlink, embed the
//! golden queries at threshold −1.0 (raw scores), persist results.
//!
//! Symlink-farm rationale: Qdrant collections are keyed
//! `ws-<sha256(workspacePath)>`. Each model indexes "its own workspace"
//! (`bench/ws-<corpus>-<model>` → corpus root), which yields one collection
//! per (corpus, model) with per-model `filePath` prefixes — collision-proof
//! even for equal dimensions. Paths are re-based to corpus-relative before
//! persisting.

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Instant;

use anyhow::Context;

use code_index::cache::HashCacheManager;
use code_index::core::search_service::normalize_directory_prefix;
use code_index::embedders::OpenAiCompatibleEmbedder;
use code_index::processors::parser::LineCodeParser;
use code_index::processors::scanner::{DirectoryScanner, ScanCallbacks};
use code_index::shared::constants::MAX_FILE_SIZE_BYTES;
use code_index::traits::{Embedder, VectorStore};
use code_index::vector_store::QdrantVectorStore;

use crate::config::{CorpusSpec, ModelSpec};
use crate::golden::GoldenSet;
use crate::server::ServerMeta;
use crate::types::{Hit, IndexStats, ModelResults, QueryResult};

/// Raw-score search: keep everything cosine can produce.
const NO_THRESHOLD: f32 = -1.0;
const RESULT_LIMIT: u32 = 50;

pub struct RunPaths {
    #[allow(dead_code)] // kept for symmetry/logging; corpus roots resolve via corpus_root()
    pub repo_root: PathBuf,
    pub bench_dir: PathBuf,
}

impl RunPaths {
    fn workspace(&self, corpus_slug: &str, model_slug: &str) -> PathBuf {
        self.bench_dir
            .join(format!("ws-{corpus_slug}-{model_slug}"))
    }
    fn tmp_dir(&self, corpus_slug: &str, model_slug: &str) -> PathBuf {
        self.bench_dir
            .join("tmp")
            .join(corpus_slug)
            .join(model_slug)
    }
    fn results_file(&self, corpus_slug: &str, model_slug: &str) -> PathBuf {
        self.bench_dir
            .join("results")
            .join(corpus_slug)
            .join(format!("{model_slug}.json"))
    }
}

/// Resolved corpus root (worktree bootstrapped if configured).
pub fn corpus_root(repo_root: &Path, corpus: &CorpusSpec) -> anyhow::Result<PathBuf> {
    let root = repo_root.join(&corpus.root);
    if let Some(branch) = &corpus.worktree_branch {
        if !root.exists() {
            let status = std::process::Command::new("git")
                .args(["worktree", "add", "--force"])
                .arg(&root)
                .arg(branch)
                .current_dir(repo_root)
                .status()
                .context("failed to run git worktree add")?;
            if !status.success() {
                anyhow::bail!("git worktree add {} {branch} failed", root.display());
            }
            println!(
                "[{}] worktree created at {} (branch {branch})",
                corpus.slug(),
                root.display()
            );
        }
    }
    if !root.exists() {
        anyhow::bail!(
            "corpus root {} does not exist (corpus \"{}\")",
            root.display(),
            corpus.name
        );
    }
    Ok(root)
}

#[allow(clippy::too_many_arguments)]
pub async fn run_model(
    paths: &RunPaths,
    corpus: &CorpusSpec,
    corpus_root: &Path,
    spec: &ModelSpec,
    meta: &ServerMeta,
    port: u16,
    golden: &GoldenSet,
    qdrant_url: &str,
) -> anyhow::Result<ModelResults> {
    let corpus_slug = corpus.slug();
    let model_slug = spec.slug();
    let ws = paths.workspace(&corpus_slug, &model_slug);
    setup_workspace(corpus_root, &ws)?;
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
    let cache_dir = paths.tmp_dir(&corpus_slug, &model_slug);
    if cache_dir.exists() {
        std::fs::remove_dir_all(&cache_dir)
            .with_context(|| format!("cannot clean {}", cache_dir.display()))?;
    }
    std::fs::create_dir_all(&cache_dir)?;
    let cache = Arc::new(HashCacheManager::new(&ws, Some(cache_dir)));

    let mut ignore_builder = ignore::gitignore::GitignoreBuilder::new(&ws);
    if corpus_root.join(".gitignore").exists() {
        ignore_builder.add(corpus_root.join(".gitignore"));
    }
    // Corpus-configured excludes (e.g. `bench/` for the self corpus so
    // golden files cannot answer their own queries).
    for pattern in &corpus.exclude {
        ignore_builder.add_line(None, pattern)?;
    }
    let matcher = ignore_builder.build()?;

    let base_url = format!("http://127.0.0.1:{port}/v1");
    // modelId override: canonical names activate product-side query-prefix
    // profiles exactly as they would for a real user's config.
    let model_id = spec
        .model_id
        .clone()
        .unwrap_or_else(|| meta.model_id.clone());
    let embedder = Arc::new(OpenAiCompatibleEmbedder::new(
        base_url,
        "bench".to_string(),
        Some(model_id),
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
        "[{corpus_slug}/{model_slug}] indexed {} files, {} blocks in {index_secs:.1}s",
        outcome.stats.processed, outcome.total_block_count
    );

    // ---- Query ----
    // Warm-up (excluded from latency): first call pays tokenizer/init costs.
    embedder
        .create_embeddings(&["warm up".to_string()], None, true)
        .await?;

    let ws_str = ws.to_string_lossy().into_owned();
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

        // --directory slice: server-side filter + client post-filter,
        // exactly the product path (SearchService + QdrantVectorStore).
        let prefix = q
            .directory
            .as_ref()
            .map(|d| normalize_directory_prefix(d, &ws));
        let found = store
            .search(
                &vector,
                prefix.as_deref(),
                Some(NO_THRESHOLD),
                Some(RESULT_LIMIT),
            )
            .await
            .context("query failed")?;
        let found: Vec<_> = match &prefix {
            Some(p) => found
                .into_iter()
                .filter(|r| {
                    r.payload
                        .as_ref()
                        .is_some_and(|pl| pl.file_path.starts_with(p.as_str()))
                })
                .collect(),
            None => found,
        };
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
                let rel = raw_path
                    .strip_prefix(ws_str.as_str())
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
        corpus: corpus.name.clone(),
        name: spec.name.clone(),
        slug: model_slug.clone(),
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

    let file = paths.results_file(&corpus_slug, &model_slug);
    if let Some(parent) = file.parent() {
        std::fs::create_dir_all(parent)?;
    }
    std::fs::write(&file, serde_json::to_vec_pretty(&results)?)
        .with_context(|| format!("cannot write {}", file.display()))?;
    println!(
        "[{corpus_slug}/{model_slug}] results written to {}",
        file.display()
    );
    Ok(results)
}

/// (Re)creates the `bench/ws-<corpus>-<model>` symlink pointing at the
/// corpus root.
fn setup_workspace(corpus_root: &Path, ws: &Path) -> anyhow::Result<()> {
    let target = corpus_root
        .canonicalize()
        .context("cannot canonicalize corpus root")?;
    if ws.symlink_metadata().is_ok() {
        std::fs::remove_file(ws).context("cannot remove stale workspace symlink")?;
    }
    std::os::unix::fs::symlink(&target, ws)
        .with_context(|| format!("symlink {} -> {}", ws.display(), target.display()))?;
    Ok(())
}

/// Deletes the model's Qdrant collection, cache dir and symlink for one
/// corpus. Never touches corpus content (e.g. worktrees under bench/tmp).
pub async fn cleanup_model(
    paths: &RunPaths,
    corpus: &CorpusSpec,
    spec: &ModelSpec,
    qdrant_url: &str,
) {
    let corpus_slug = corpus.slug();
    let model_slug = spec.slug();
    let ws = paths.workspace(&corpus_slug, &model_slug);
    let store = QdrantVectorStore::new(&ws.to_string_lossy(), Some(qdrant_url), 1, None);
    store.delete_collection().await.ok();
    if let Err(err) = std::fs::remove_dir_all(paths.tmp_dir(&corpus_slug, &model_slug)) {
        if err.kind() != std::io::ErrorKind::NotFound {
            eprintln!("[{corpus_slug}/{model_slug}] cache cleanup failed: {err}");
        }
    }
    let _ = std::fs::remove_file(&ws);
}
