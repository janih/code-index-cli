//! Directory scanner: walks files, parses code blocks, embeds and upserts.
//!
//! Port of `src/processors/scanner.ts`. p-limit/async-mutex map to
//! `buffer_unordered` (parse concurrency), a `JoinSet` (pending batches) and
//! a `Semaphore` (batch concurrency). gitignore matching comes from the
//! `ignore` crate via a matcher provided by the service factory.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use futures::stream::{self, StreamExt};
use ignore::gitignore::Gitignore;
use sha2::{Digest, Sha256};
use tokio::task::JoinSet;
use uuid::Uuid;

use crate::log;
use crate::shared::cancellation::CancellationToken;
use crate::shared::constants::{
    BATCH_PROCESSING_CONCURRENCY, BATCH_SEGMENT_THRESHOLD, MAX_FILE_SIZE_BYTES,
    MAX_LIST_FILES_LIMIT, MAX_PENDING_BATCHES, PARSING_CONCURRENCY, QDRANT_CODE_BLOCK_NAMESPACE,
};
use crate::shared::supported_extensions::is_supported_extension;
use crate::traits::{CacheManager, CodeParser, Embedder, PointStruct, VectorStore};

/// Directories always skipped during scanning (at walk time).
pub const IGNORED_DIRECTORIES: [&str; 13] = [
    "node_modules",
    ".git",
    "dist",
    "build",
    ".next",
    ".nuxt",
    "target",
    "__pycache__",
    ".cache",
    ".turbo",
    "coverage",
    ".venv",
    "venv",
];

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct ScanStats {
    pub processed: usize,
    pub skipped: usize,
}

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct ScanOutcome {
    pub stats: ScanStats,
    pub total_block_count: usize,
}

/// Optional progress/error callbacks. `Arc`-wrapped so batch tasks can
/// report errors after being spawned.
#[derive(Default, Clone)]
pub struct ScanCallbacks {
    pub on_error: Option<Arc<dyn Fn(anyhow::Error) + Send + Sync>>,
    pub on_blocks_indexed: Option<Arc<dyn Fn(usize) + Send + Sync>>,
    pub on_file_parsed: Option<Arc<dyn Fn(usize) + Send + Sync>>,
}

/// Deterministic Qdrant point id for a code block: uuid-v5 over the segment
/// hash (exactly what the TS scanner computes).
pub fn point_id_for_segment(segment_hash: &str) -> Uuid {
    let namespace = Uuid::parse_str(QDRANT_CODE_BLOCK_NAMESPACE)
        .expect("namespace constant must be a valid UUID");
    Uuid::new_v5(&namespace, segment_hash.as_bytes())
}

/// Builds the indexed point for one code block: deterministic uuid-v5 id
/// plus the payload schema (filePath/codeChunk/startLine/endLine/
/// segmentHash/fileHash). Shared by the scanner and the watcher so the two
/// indexing paths cannot drift apart.
pub fn block_to_point(block: &crate::traits::CodeBlock, vector: Vec<f32>) -> PointStruct {
    let mut payload = serde_json::Map::new();
    payload.insert("filePath".into(), block.file_path.clone().into());
    payload.insert("codeChunk".into(), block.content.clone().into());
    payload.insert("startLine".into(), (block.start_line as u64).into());
    payload.insert("endLine".into(), (block.end_line as u64).into());
    payload.insert("segmentHash".into(), block.segment_hash.clone().into());
    payload.insert("fileHash".into(), block.file_hash.clone().into());
    PointStruct {
        id: point_id_for_segment(&block.segment_hash).to_string(),
        vector,
        payload,
    }
}

/// Result of handling one file during scanning.
enum FileOutcome {
    /// Skipped: too large or unchanged since last scan.
    Skipped,
    /// Parsed successfully.
    Processed {
        file_path: String,
        file_hash: String,
        blocks: Vec<crate::traits::CodeBlock>,
    },
    /// Failed to process; scanning continues with other files.
    Errored(anyhow::Error),
}

/// Directory scanner that walks a workspace directory and indexes code files.
pub struct DirectoryScanner {
    embedder: Arc<dyn Embedder>,
    vector_store: Arc<dyn VectorStore>,
    code_parser: Arc<dyn CodeParser>,
    cache_manager: Arc<dyn CacheManager>,
    ignore_matcher: Gitignore,
    batch_segment_threshold: usize,
    max_file_size_bytes: u64,
}

impl DirectoryScanner {
    pub fn new(
        embedder: Arc<dyn Embedder>,
        vector_store: Arc<dyn VectorStore>,
        code_parser: Arc<dyn CodeParser>,
        cache_manager: Arc<dyn CacheManager>,
        ignore_matcher: Gitignore,
        batch_segment_threshold: Option<usize>,
        max_file_size_bytes: Option<u64>,
    ) -> Self {
        Self {
            embedder,
            vector_store,
            code_parser,
            cache_manager,
            ignore_matcher,
            batch_segment_threshold: batch_segment_threshold.unwrap_or(BATCH_SEGMENT_THRESHOLD),
            max_file_size_bytes: max_file_size_bytes.unwrap_or(MAX_FILE_SIZE_BYTES),
        }
    }

    /// Recursively scans a directory for code blocks in supported files.
    pub async fn scan_directory(
        &self,
        directory: &Path,
        callbacks: ScanCallbacks,
        signal: Option<CancellationToken>,
    ) -> anyhow::Result<ScanOutcome> {
        let supported_paths = self.list_supported_files(directory);

        let mut processed_count = 0usize;
        let mut skipped_count = 0usize;
        let mut total_block_count = 0usize;

        let mut current_batch_blocks: Vec<crate::traits::CodeBlock> = Vec::new();
        let mut pending_batches: JoinSet<anyhow::Result<usize>> = JoinSet::new();
        let batch_semaphore = Arc::new(tokio::sync::Semaphore::new(BATCH_PROCESSING_CONCURRENCY));

        let cancelled = signal.clone();
        let cache = Arc::clone(&self.cache_manager);
        let parser = Arc::clone(&self.code_parser);
        let max_file_size = self.max_file_size_bytes;

        let outcomes = stream::iter(supported_paths)
            .map(move |path| {
                let cache = Arc::clone(&cache);
                let parser = Arc::clone(&parser);
                let token = cancelled.clone();
                async move {
                    if token.as_ref().is_some_and(|t| t.is_cancelled()) {
                        return None;
                    }
                    Some(Self::scan_one_file(path, cache, parser, max_file_size).await)
                }
            })
            .buffer_unordered(PARSING_CONCURRENCY);
        tokio::pin!(outcomes);

        while let Some(outcome) = outcomes.next().await {
            let Some(outcome) = outcome else { continue };
            match outcome {
                FileOutcome::Skipped => skipped_count += 1,
                FileOutcome::Errored(err) => {
                    if let Some(on_error) = &callbacks.on_error {
                        on_error(err);
                    }
                }
                FileOutcome::Processed {
                    file_path,
                    file_hash,
                    blocks,
                } => {
                    if let Some(on_file_parsed) = &callbacks.on_file_parsed {
                        on_file_parsed(blocks.len());
                    }
                    processed_count += 1;

                    if !blocks.is_empty() {
                        if signal.as_ref().is_some_and(|t| t.is_cancelled()) {
                            break;
                        }

                        // Delete stale points for this file BEFORE queueing
                        // its new blocks. The TS code deleted after upserting
                        // (wiping the fresh points); delete-first keeps the
                        // index consistent — see AGENTS.md deviations.
                        //
                        // Skipped when the file has no cached hash: a
                        // never-indexed file has no stale points, and the
                        // unconditional delete cost one HTTP round-trip per
                        // file on every initial scan. Cache and collection
                        // are created/cleared together, so a missing hash
                        // implies missing points — see the AGENTS.md
                        // deviations caveat about lost cache files.
                        if self.cache_manager.get_hash(&file_path).is_some() {
                            if let Err(err) = self
                                .vector_store
                                .delete_points_by_multiple_file_paths(std::slice::from_ref(
                                    &file_path,
                                ))
                                .await
                            {
                                crate::log::error(&format!("Failed to delete stale points: {err}"));
                            }
                        }

                        for block in blocks {
                            if block.content.trim().is_empty() {
                                continue;
                            }
                            current_batch_blocks.push(block);

                            if current_batch_blocks.len() >= self.batch_segment_threshold {
                                // Backpressure: wait until there is headroom for another pending batch
                                while pending_batches.len() >= MAX_PENDING_BATCHES {
                                    total_block_count = Self::await_one_batch(
                                        &mut pending_batches,
                                        total_block_count,
                                        &callbacks,
                                    )
                                    .await;
                                    if signal.as_ref().is_some_and(|t| t.is_cancelled()) {
                                        break;
                                    }
                                }

                                let batch_blocks = std::mem::take(&mut current_batch_blocks);
                                Self::spawn_batch(
                                    &mut pending_batches,
                                    &batch_semaphore,
                                    self.embedder.clone(),
                                    self.vector_store.clone(),
                                    batch_blocks,
                                    callbacks.on_error.clone(),
                                );
                            }
                        }

                        self.cache_manager.update_hash(&file_path, file_hash);
                    }
                }
            }
        }

        // Process remaining batch
        if !current_batch_blocks.is_empty() {
            match Self::process_batch(
                self.embedder.clone(),
                self.vector_store.clone(),
                std::mem::take(&mut current_batch_blocks),
                callbacks.on_error.clone(),
            )
            .await
            {
                Ok(indexed) => {
                    total_block_count += indexed;
                    if let Some(on_blocks_indexed) = &callbacks.on_blocks_indexed {
                        on_blocks_indexed(indexed);
                    }
                }
                Err(err) => return Err(err),
            }
        }

        // Wait for all pending batches
        while !pending_batches.is_empty() {
            total_block_count =
                Self::await_one_batch(&mut pending_batches, total_block_count, &callbacks).await;
        }

        Ok(ScanOutcome {
            stats: ScanStats {
                processed: processed_count,
                skipped: skipped_count,
            },
            total_block_count,
        })
    }

    /// Joins one pending batch task, folding its result into the totals.
    /// Batch errors are logged and reported via on_error (matched against the
    /// TS behavior where each processBatch error surfaces via onError).
    async fn await_one_batch(
        pending: &mut JoinSet<anyhow::Result<usize>>,
        total: usize,
        callbacks: &ScanCallbacks,
    ) -> usize {
        match pending.join_next().await {
            Some(Ok(Ok(indexed))) => {
                if let Some(on_blocks_indexed) = &callbacks.on_blocks_indexed {
                    on_blocks_indexed(indexed);
                }
                total + indexed
            }
            Some(Ok(Err(err))) => {
                log::error(&format!("Error processing batch: {err}"));
                if let Some(on_error) = &callbacks.on_error {
                    on_error(err);
                }
                total
            }
            Some(Err(join_err)) => {
                log::error(&format!("Batch task failed: {join_err}"));
                total
            }
            None => total,
        }
    }

    fn spawn_batch(
        pending: &mut JoinSet<anyhow::Result<usize>>,
        semaphore: &Arc<tokio::sync::Semaphore>,
        embedder: Arc<dyn Embedder>,
        vector_store: Arc<dyn VectorStore>,
        blocks: Vec<crate::traits::CodeBlock>,
        on_error: Option<Arc<dyn Fn(anyhow::Error) + Send + Sync>>,
    ) {
        let permit = semaphore.clone();
        pending.spawn(async move {
            let _permit = permit.acquire().await.expect("batch semaphore closed");
            Self::process_batch(embedder, vector_store, blocks, on_error).await
        });
    }

    /// Processes a batch of code blocks: embed and upsert.
    async fn process_batch(
        embedder: Arc<dyn Embedder>,
        vector_store: Arc<dyn VectorStore>,
        blocks: Vec<crate::traits::CodeBlock>,
        on_error: Option<Arc<dyn Fn(anyhow::Error) + Send + Sync>>,
    ) -> anyhow::Result<usize> {
        let result = async {
            let texts: Vec<String> = blocks
                .iter()
                .map(|b| b.content.trim().to_string())
                .collect();
            let count = texts.len();
            let embedding_response = embedder.create_embeddings(&texts, None, false).await?;

            // A provider returning fewer embeddings than inputs must fail the
            // batch loudly — zipping would silently drop the tail blocks.
            let embeddings = embedding_response.embeddings;
            if embeddings.len() != count {
                anyhow::bail!(
                    "Embedder returned {} embeddings for {} texts; refusing to upsert a partial batch",
                    embeddings.len(),
                    count
                );
            }

            let points: Vec<PointStruct> = blocks
                .into_iter()
                .zip(embeddings)
                .map(|(block, vector)| block_to_point(&block, vector))
                .collect();

            vector_store.upsert_points(points).await?;
            Ok(count)
        }
        .await;

        match result {
            Ok(indexed) => Ok(indexed),
            Err(err) => {
                log::error(&format!("Error processing batch: {err}"));
                if let Some(on_error) = &on_error {
                    on_error(anyhow::anyhow!("{err:#}"));
                }
                Err(err)
            }
        }
    }

    async fn scan_one_file(
        path: PathBuf,
        cache_manager: Arc<dyn CacheManager>,
        code_parser: Arc<dyn CodeParser>,
        max_file_size: u64,
    ) -> FileOutcome {
        let file_path = path.to_string_lossy().into_owned();
        let block_file_path = file_path.clone();
        let result: anyhow::Result<FileOutcome> = async move {
            let metadata = std::fs::metadata(&path)?;
            if metadata.len() > max_file_size {
                return Ok(FileOutcome::Skipped);
            }

            let content = std::fs::read_to_string(&path)?;
            let current_file_hash = hex::encode(Sha256::digest(content.as_bytes()));

            if cache_manager.get_hash(&block_file_path).as_deref()
                == Some(current_file_hash.as_str())
            {
                return Ok(FileOutcome::Skipped);
            }

            let blocks = code_parser
                .parse_file(
                    &path,
                    Some(crate::traits::ParseOptions {
                        content: Some(content),
                        file_hash: Some(current_file_hash.clone()),
                        ..Default::default()
                    }),
                )
                .await?;

            Ok(FileOutcome::Processed {
                file_path: block_file_path,
                file_hash: current_file_hash,
                blocks,
            })
        }
        .await;

        result.unwrap_or_else(|err| {
            FileOutcome::Errored(err.context(format!("Error processing file {file_path}")))
        })
    }

    /// Lists all files in a directory recursively, skipping IGNORED_DIRECTORIES
    /// at walk time and capped at MAX_LIST_FILES_LIMIT. Symlinks are not
    /// followed (same as the TS Dirent-based walk).
    fn list_supported_files(&self, directory: &Path) -> Vec<PathBuf> {
        let mut results = Vec::new();
        let mut stack = vec![directory.to_path_buf()];

        while let Some(dir) = stack.pop() {
            if results.len() as u64 >= MAX_LIST_FILES_LIMIT {
                break;
            }
            let Ok(entries) = std::fs::read_dir(&dir) else {
                continue;
            };
            for entry in entries.flatten() {
                if results.len() as u64 >= MAX_LIST_FILES_LIMIT {
                    break;
                }
                let path = entry.path();
                let Ok(file_type) = entry.file_type() else {
                    continue;
                };
                if file_type.is_symlink() {
                    continue;
                }
                if file_type.is_dir() {
                    if !IGNORED_DIRECTORIES.contains(&entry.file_name().to_string_lossy().as_ref())
                    {
                        stack.push(path);
                    }
                } else if file_type.is_file() {
                    results.push(path);
                }
            }
        }

        results.retain(|file_path| {
            let ext = file_path
                .extension()
                .and_then(|e| e.to_str())
                .map(|e| format!(".{}", e.to_lowercase()));
            let Some(ext) = ext else { return false };
            if !is_supported_extension(&ext) {
                return false;
            }
            if let Ok(relative) = file_path.strip_prefix(directory) {
                !crate::shared::ignore_match::is_ignored(&self.ignore_matcher, relative, false)
            } else {
                true
            }
        });

        results
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::traits::{EmbeddingResponse, ValidationResult};
    use std::collections::HashMap;
    use std::sync::Mutex;
    use std::time::{SystemTime, UNIX_EPOCH};

    // ---------------------------------------------------------------
    // Mocks (same seams the TS scanner tests mock)
    // ---------------------------------------------------------------

    struct MockEmbedder {
        dimension: usize,
    }

    #[async_trait::async_trait]
    impl Embedder for MockEmbedder {
        async fn create_embeddings(
            &self,
            texts: &[String],
            _model: Option<&str>,
            _is_query: bool,
        ) -> anyhow::Result<EmbeddingResponse> {
            Ok(EmbeddingResponse {
                embeddings: texts.iter().map(|_| vec![0.1f32; self.dimension]).collect(),
                usage: None,
            })
        }

        async fn validate_configuration(&self) -> ValidationResult {
            ValidationResult::ok()
        }

        fn embedder_info(&self) -> crate::traits::EmbedderInfo {
            crate::traits::EmbedderInfo {
                name: crate::shared::embedding_models::EmbedderProvider::OpenAi,
            }
        }
    }

    /// Returns one embedding fewer than requested — simulates a provider
    /// that drops oversized items (the OpenAI batching plan can do this).
    struct TruncatingEmbedder;

    #[async_trait::async_trait]
    impl Embedder for TruncatingEmbedder {
        async fn create_embeddings(
            &self,
            texts: &[String],
            _model: Option<&str>,
            _is_query: bool,
        ) -> anyhow::Result<EmbeddingResponse> {
            let mut embeddings: Vec<Vec<f32>> = texts.iter().map(|_| vec![0.1f32; 8]).collect();
            embeddings.pop();
            Ok(EmbeddingResponse {
                embeddings,
                usage: None,
            })
        }

        async fn validate_configuration(&self) -> ValidationResult {
            ValidationResult::ok()
        }

        fn embedder_info(&self) -> crate::traits::EmbedderInfo {
            crate::traits::EmbedderInfo {
                name: crate::shared::embedding_models::EmbedderProvider::OpenAi,
            }
        }
    }

    struct MockVectorStore {
        upserted: Mutex<Vec<Vec<PointStruct>>>,
        deleted_paths: Mutex<Vec<String>>,
        ops: Mutex<Vec<String>>,
    }

    impl MockVectorStore {
        fn new() -> Self {
            Self {
                upserted: Mutex::new(Vec::new()),
                deleted_paths: Mutex::new(Vec::new()),
                ops: Mutex::new(Vec::new()),
            }
        }
    }

    #[async_trait::async_trait]
    impl VectorStore for MockVectorStore {
        async fn initialize(&self) -> anyhow::Result<bool> {
            Ok(true)
        }

        async fn upsert_points(&self, points: Vec<PointStruct>) -> anyhow::Result<()> {
            let files: Vec<&str> = points
                .iter()
                .filter_map(|p| p.payload.get("filePath").and_then(|v| v.as_str()))
                .collect();
            self.ops
                .lock()
                .unwrap()
                .push(format!("upsert:{}", files.join(",")));
            self.upserted.lock().unwrap().push(points);
            Ok(())
        }

        async fn search(
            &self,
            _query: &[f32],
            _prefix: Option<&str>,
            _min_score: Option<f32>,
            _max: Option<u32>,
        ) -> anyhow::Result<Vec<crate::traits::VectorStoreSearchResult>> {
            Ok(Vec::new())
        }

        async fn delete_points_by_file_path(&self, _file_path: &str) -> anyhow::Result<()> {
            Ok(())
        }

        async fn delete_points_by_multiple_file_paths(
            &self,
            file_paths: &[String],
        ) -> anyhow::Result<()> {
            self.deleted_paths
                .lock()
                .unwrap()
                .extend_from_slice(file_paths);
            Ok(())
        }

        async fn clear_collection(&self) -> anyhow::Result<()> {
            Ok(())
        }
        async fn delete_collection(&self) -> anyhow::Result<()> {
            Ok(())
        }
        async fn collection_exists(&self) -> anyhow::Result<bool> {
            Ok(true)
        }
        async fn has_indexed_data(&self) -> anyhow::Result<bool> {
            Ok(true)
        }
        async fn mark_indexing_complete(&self) -> anyhow::Result<()> {
            Ok(())
        }
        async fn mark_indexing_incomplete(&self) -> anyhow::Result<()> {
            Ok(())
        }
    }

    struct MockCache {
        hashes: Mutex<HashMap<String, String>>,
    }

    impl MockCache {
        fn new() -> Self {
            Self {
                hashes: Mutex::new(HashMap::new()),
            }
        }
    }

    #[async_trait::async_trait]
    impl CacheManager for MockCache {
        fn get_hash(&self, file_path: &str) -> Option<String> {
            self.hashes.lock().unwrap().get(file_path).cloned()
        }
        fn update_hash(&self, file_path: &str, hash: String) {
            self.hashes
                .lock()
                .unwrap()
                .insert(file_path.to_string(), hash);
        }
        fn delete_hash(&self, file_path: &str) {
            self.hashes.lock().unwrap().remove(file_path);
        }
        async fn flush(&self) -> anyhow::Result<()> {
            Ok(())
        }
        fn get_all_hashes(&self) -> HashMap<String, String> {
            self.hashes.lock().unwrap().clone()
        }
    }

    // ---------------------------------------------------------------
    // Helpers
    // ---------------------------------------------------------------

    fn temp_workspace(tag: &str) -> PathBuf {
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let dir = std::env::temp_dir().join(format!("code-index-scanner-test-{tag}-{nanos}"));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn file_block_lines(n: usize) -> String {
        (1..=n)
            .map(|i| format!("// line {i} of this sufficiently long source file"))
            .collect::<Vec<_>>()
            .join("\n")
    }

    struct Fixture {
        scanner: DirectoryScanner,
        store: Arc<MockVectorStore>,
    }

    fn fixture() -> Fixture {
        let store = Arc::new(MockVectorStore::new());
        let cache = Arc::new(MockCache::new());
        let scanner = DirectoryScanner::new(
            Arc::new(MockEmbedder { dimension: 8 }),
            store.clone(),
            Arc::new(crate::processors::parser::LineCodeParser::new()),
            cache.clone(),
            Gitignore::empty(),
            None,
            None,
        );
        Fixture { scanner, store }
    }

    // ---------------------------------------------------------------
    // Tests
    // ---------------------------------------------------------------

    #[tokio::test]
    async fn indexes_supported_files() {
        let dir = temp_workspace("basic");
        std::fs::write(dir.join("a.ts"), file_block_lines(10)).unwrap();
        std::fs::write(dir.join("b.py"), file_block_lines(10)).unwrap();
        std::fs::write(dir.join("ignore.xyz"), file_block_lines(10)).unwrap();

        let f = fixture();
        let outcome = f
            .scanner
            .scan_directory(&dir, ScanCallbacks::default(), None)
            .await
            .unwrap();

        assert_eq!(outcome.stats.processed, 2);
        assert_eq!(outcome.stats.skipped, 0);
        assert_eq!(outcome.total_block_count, 2);
        assert!(!f.store.upserted.lock().unwrap().is_empty());
        // Fresh scan: no cached hashes => no stale-point deletes (review #10)
        assert!(f.store.deleted_paths.lock().unwrap().is_empty());
    }

    #[tokio::test]
    async fn skips_unchanged_files_via_cache() {
        let dir = temp_workspace("cache");
        let path = dir.join("a.ts");
        std::fs::write(&path, file_block_lines(10)).unwrap();

        let f = fixture();
        let first = f
            .scanner
            .scan_directory(&dir, ScanCallbacks::default(), None)
            .await
            .unwrap();
        assert_eq!(first.stats.processed, 1);

        let second = f
            .scanner
            .scan_directory(&dir, ScanCallbacks::default(), None)
            .await
            .unwrap();
        assert_eq!(second.stats.skipped, 1);
        assert_eq!(second.stats.processed, 0);
    }

    #[tokio::test]
    async fn reindexes_changed_files_and_deletes_stale_points() {
        let dir = temp_workspace("changed");
        let path = dir.join("a.ts");
        std::fs::write(&path, file_block_lines(10)).unwrap();

        let f = fixture();
        f.scanner
            .scan_directory(&dir, ScanCallbacks::default(), None)
            .await
            .unwrap();

        std::fs::write(&path, file_block_lines(12)).unwrap();
        let before = f.store.deleted_paths.lock().unwrap().len();
        let outcome = f
            .scanner
            .scan_directory(&dir, ScanCallbacks::default(), None)
            .await
            .unwrap();
        assert_eq!(outcome.stats.processed, 1);
        let deleted = f.store.deleted_paths.lock().unwrap();
        assert_eq!(deleted.len(), before + 1);
        assert!(deleted.last().unwrap().ends_with("a.ts"));
    }

    #[tokio::test]
    async fn deletes_stale_points_before_upserting_new_ones() {
        let dir = temp_workspace("order");
        let path = dir.join("a.ts");
        std::fs::write(&path, file_block_lines(10)).unwrap();

        let f = fixture();
        f.scanner
            .scan_directory(&dir, ScanCallbacks::default(), None)
            .await
            .unwrap();
        std::fs::write(&path, file_block_lines(12)).unwrap();
        // Fresh scan recorded only upserts (no cached hashes yet); reset the
        // op log so the ordering assertion below covers the re-index only.
        f.store.ops.lock().unwrap().clear();
        f.scanner
            .scan_directory(&dir, ScanCallbacks::default(), None)
            .await
            .unwrap();

        let ops = f.store.ops.lock().unwrap();
        let upserts: Vec<usize> = ops
            .iter()
            .enumerate()
            .filter(|(_, op)| op.starts_with("upsert:"))
            .map(|(i, _)| i)
            .collect();
        let deletes: Vec<usize> = ops
            .iter()
            .enumerate()
            .filter(|(_, op)| op.starts_with("delete:"))
            .map(|(i, _)| i)
            .collect();
        // Every upsert of a file's points is preceded by that file's delete
        for (upsert_idx, delete_idx) in upserts.iter().zip(deletes.iter()) {
            assert!(
                delete_idx < upsert_idx,
                "delete@{delete_idx} must precede upsert@{upsert_idx}: {ops:?}"
            );
        }
    }

    #[tokio::test]
    async fn skips_files_above_configured_max_size() {
        let dir = temp_workspace("custom-max");
        std::fs::write(dir.join("big.ts"), "x".repeat(300)).unwrap();
        // ~80 bytes: above MIN_BLOCK_CHARS so it still indexes
        std::fs::write(
            dir.join("small.ts"),
            format!("fn a() {{ {} }}", "y".repeat(60)),
        )
        .unwrap();

        let store = Arc::new(MockVectorStore::new());
        let scanner = DirectoryScanner::new(
            Arc::new(MockEmbedder { dimension: 8 }),
            store,
            Arc::new(crate::processors::parser::LineCodeParser::new()),
            Arc::new(MockCache::new()),
            Gitignore::empty(),
            None,
            Some(200), // custom indexing.maxFileSizeBytes
        );

        let outcome = scanner
            .scan_directory(&dir, ScanCallbacks::default(), None)
            .await
            .unwrap();
        assert_eq!(outcome.stats.skipped, 1);
        assert_eq!(outcome.stats.processed, 1);
    }

    #[tokio::test]
    async fn skips_oversized_files() {
        let dir = temp_workspace("size");
        let content = "x".repeat((MAX_FILE_SIZE_BYTES + 1) as usize);
        std::fs::write(dir.join("big.ts"), content).unwrap();

        let f = fixture();
        let outcome = f
            .scanner
            .scan_directory(&dir, ScanCallbacks::default(), None)
            .await
            .unwrap();
        assert_eq!(outcome.stats.skipped, 1);
        assert_eq!(outcome.stats.processed, 0);
    }

    #[tokio::test]
    async fn skips_ignored_directories_at_walk_time() {
        let dir = temp_workspace("walk");
        let nested = dir.join("node_modules").join("pkg");
        std::fs::create_dir_all(&nested).unwrap();
        std::fs::write(nested.join("index.ts"), file_block_lines(10)).unwrap();
        std::fs::write(dir.join("src.ts"), file_block_lines(10)).unwrap();

        let f = fixture();
        let outcome = f
            .scanner
            .scan_directory(&dir, ScanCallbacks::default(), None)
            .await
            .unwrap();
        assert_eq!(outcome.stats.processed, 1);
    }

    #[tokio::test]
    async fn batches_flush_at_threshold() {
        let dir = temp_workspace("batches");
        // Every ~20-line file = 1 block; 130 files over threshold 60 => 3 batches
        for i in 0..130 {
            std::fs::write(dir.join(format!("f{i}.ts")), file_block_lines(20)).unwrap();
        }

        let f = fixture();
        let outcome = f
            .scanner
            .scan_directory(&dir, ScanCallbacks::default(), None)
            .await
            .unwrap();
        assert_eq!(outcome.total_block_count, 130);
        let batch_sizes: Vec<usize> = f
            .store
            .upserted
            .lock()
            .unwrap()
            .iter()
            .map(|batch| batch.len())
            .collect();
        assert_eq!(batch_sizes.iter().sum::<usize>(), 130);
        // 60 + 60 + 10 (remainder)
        assert_eq!(batch_sizes.len(), 3);
    }

    #[tokio::test]
    async fn embedding_count_mismatch_fails_batch_loudly() {
        // Exactly 60 blocks = one spawned batch (soft-error path via
        // on_error); a remainder would take the inline hard-error path.
        let dir = temp_workspace("mismatch");
        for i in 0..60 {
            std::fs::write(dir.join(format!("f{i}.ts")), file_block_lines(20)).unwrap();
        }

        let store = Arc::new(MockVectorStore::new());
        let scanner = DirectoryScanner::new(
            Arc::new(TruncatingEmbedder),
            store.clone(),
            Arc::new(crate::processors::parser::LineCodeParser::new()),
            Arc::new(MockCache::new()),
            Gitignore::empty(),
            None,
            None,
        );

        let errors: Arc<std::sync::Mutex<Vec<String>>> =
            Arc::new(std::sync::Mutex::new(Vec::new()));
        let sink = Arc::clone(&errors);
        let callbacks = ScanCallbacks {
            on_error: Some(Arc::new(move |err| {
                sink.lock().unwrap().push(err.to_string());
            })),
            ..Default::default()
        };

        let outcome = scanner.scan_directory(&dir, callbacks, None).await.unwrap();

        // Nothing was upserted and the mismatch surfaced via on_error —
        // previously the zip silently dropped one block per batch.
        assert_eq!(outcome.total_block_count, 0);
        assert!(store.upserted.lock().unwrap().is_empty());
        let errors = errors.lock().unwrap();
        assert!(
            errors
                .iter()
                .any(|e| e.contains("refusing to upsert a partial batch")),
            "expected mismatch error, got: {errors:?}"
        );
    }

    #[tokio::test]
    async fn point_ids_are_deterministic_uuids() {
        let dir = temp_workspace("ids");
        std::fs::write(dir.join("a.ts"), file_block_lines(10)).unwrap();

        let f = fixture();
        f.scanner
            .scan_directory(&dir, ScanCallbacks::default(), None)
            .await
            .unwrap();
        let batches = f.store.upserted.lock().unwrap();
        let id = batches[0][0].id.clone();
        assert_eq!(
            Uuid::parse_str(&id).unwrap().get_version(),
            Some(uuid::Version::Sha1)
        );
    }

    #[tokio::test]
    async fn cancellation_stops_early() {
        let dir = temp_workspace("cancel");
        for i in 0..50 {
            std::fs::write(dir.join(format!("f{i}.ts")), file_block_lines(10)).unwrap();
        }

        let token = CancellationToken::new();
        token.cancel(); // cancelled before the scan starts

        let f = fixture();
        let outcome = f
            .scanner
            .scan_directory(&dir, ScanCallbacks::default(), Some(token))
            .await
            .unwrap();
        assert_eq!(outcome.stats.processed, 0);
    }

    #[test]
    fn point_id_matches_rfc4122_v5() {
        // Locked against Python uuid5(ns, hash) — see qdrant.rs test
        let id = point_id_for_segment("a".repeat(64).as_str());
        assert_eq!(id.get_version(), Some(uuid::Version::Sha1));
    }

    #[test]
    fn block_to_point_payload_matches_index_schema() {
        let block = crate::traits::CodeBlock {
            file_path: "src/a.ts".into(),
            content: "fn main() {}".into(),
            start_line: 3,
            end_line: 4,
            segment_hash: "abc".into(),
            file_hash: "def".into(),
        };
        let point = block_to_point(&block, vec![0.25]);
        assert_eq!(point.id, point_id_for_segment("abc").to_string());
        assert_eq!(point.payload["filePath"], serde_json::json!("src/a.ts"));
        assert_eq!(
            point.payload["codeChunk"],
            serde_json::json!("fn main() {}")
        );
        assert_eq!(point.payload["startLine"], serde_json::json!(3));
        assert_eq!(point.payload["endLine"], serde_json::json!(4));
        assert_eq!(point.payload["segmentHash"], serde_json::json!("abc"));
        assert_eq!(point.payload["fileHash"], serde_json::json!("def"));
        assert_eq!(point.payload.len(), 6);
        assert_eq!(point.vector, vec![0.25]);
    }
}
