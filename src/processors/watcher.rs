//! File watcher: debounced incremental reindexing.
//!
//! Port of `src/processors/file-watcher.ts`. chokidar becomes `notify`,
//! the 500ms debounce and per-batch processing semantics are preserved.
//!
//! Note: `awaitWriteFinish` has no native equivalent — the hash-cache check
//! plus subsequent change events converge the index to the settled content.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use notify::event::{ModifyKind, RenameMode};
use notify::{Event, EventKind, RecommendedWatcher, RecursiveMode, Watcher};
use serde_json::Map;
use sha2::{Digest, Sha256};
use tokio::sync::mpsc;
use uuid::Uuid;

use crate::log;
use crate::shared::constants::{
    BATCH_SEGMENT_THRESHOLD, MAX_FILE_SIZE_BYTES, QDRANT_CODE_BLOCK_NAMESPACE,
};
use crate::shared::supported_extensions::is_supported_extension;
use crate::traits::{CacheManager, CodeParser, Embedder, PointStruct, VectorStore};

use super::scanner::IGNORED_DIRECTORIES;

/// Progress callback for per-file results within a batch.
pub type FileProgressCallback<'a> = Option<&'a dyn Fn(usize, usize, &Path, &FileProcessingResult)>;

/// Accumulated fs event (TS: accumulatedEvents).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WatchEventType {
    Create,
    Change,
    Delete,
}

/// Outcome for one file in a batch (TS: FileProcessingResult).
#[derive(Debug, Clone, PartialEq)]
pub struct FileProcessingResult {
    pub path: PathBuf,
    pub status: FileStatus,
    pub reason: Option<String>,
    pub new_hash: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FileStatus {
    Success,
    Skipped,
    Error,
}

/// Summary of one debounced batch (TS: BatchProcessingSummary).
#[derive(Debug, Default)]
pub struct BatchSummary {
    pub processed_files: Vec<FileProcessingResult>,
    pub batch_error: Option<String>,
}

impl BatchSummary {
    pub fn success_count(&self) -> usize {
        self.processed_files
            .iter()
            .filter(|f| f.status == FileStatus::Success)
            .count()
    }

    pub fn error_count(&self) -> usize {
        self.processed_files
            .iter()
            .filter(|f| f.status == FileStatus::Error)
            .count()
    }
}

/// File watcher: incremental processing of create/change/delete events.
pub struct FileWatcher {
    workspace_path: PathBuf,
    /// Canonical form of workspace_path: on macOS FSEvents reports the real
    /// path (e.g. /private/tmp/...) even when watching a symlinked path, so
    /// event paths must be rebased onto the watched root for the stored
    /// filePath strings to match the scanner's.
    workspace_canonical: PathBuf,
    cache_manager: Arc<dyn CacheManager>,
    embedder: Option<Arc<dyn Embedder>>,
    vector_store: Option<Arc<dyn VectorStore>>,
    ignore_matcher: ignore::gitignore::Gitignore,
    code_parser: Arc<dyn CodeParser>,
    #[allow(dead_code)]
    batch_segment_threshold: usize,
}

impl FileWatcher {
    pub fn new(
        workspace_path: PathBuf,
        cache_manager: Arc<dyn CacheManager>,
        embedder: Option<Arc<dyn Embedder>>,
        vector_store: Option<Arc<dyn VectorStore>>,
        ignore_matcher: ignore::gitignore::Gitignore,
        batch_segment_threshold: Option<usize>,
    ) -> Self {
        let workspace_canonical = workspace_path
            .canonicalize()
            .unwrap_or_else(|_| workspace_path.clone());
        Self {
            workspace_path,
            workspace_canonical,
            cache_manager,
            embedder,
            vector_store,
            ignore_matcher,
            code_parser: Arc::new(crate::processors::parser::LineCodeParser::new()),
            batch_segment_threshold: batch_segment_threshold.unwrap_or(BATCH_SEGMENT_THRESHOLD),
        }
    }

    /// Starts the OS watcher, returning the watcher (keepalive) + event stream.
    ///
    /// Watches the workspace recursively; filtering of extensions, ignored
    /// directories and gitignore rules happens per event (the chokidar glob
    /// becomes explicit predicates).
    pub fn start_notify_stream(
        &self,
    ) -> anyhow::Result<(RecommendedWatcher, mpsc::UnboundedReceiver<Event>)> {
        let (tx, rx) = mpsc::unbounded_channel();
        let mut watcher = notify::recommended_watcher(
            move |result: Result<Event, notify::Error>| match result {
                Ok(event) => {
                    let _ = tx.send(event);
                }
                Err(err) => log::error(&format!("File watcher error: {err}")),
            },
        )?;
        watcher.watch(&self.workspace_path, RecursiveMode::Recursive)?;
        Ok((watcher, rx))
    }

    /// Classifies an fs event path: the watch event type if relevant.
    pub fn classify_event(&self, event: &Event) -> Vec<(PathBuf, WatchEventType)> {
        let event_type = match event.kind {
            EventKind::Create(_) => Some(WatchEventType::Create),
            EventKind::Modify(ModifyKind::Name(RenameMode::From)) => Some(WatchEventType::Delete),
            EventKind::Modify(ModifyKind::Name(RenameMode::To)) => Some(WatchEventType::Create),
            EventKind::Modify(_) => Some(WatchEventType::Change),
            EventKind::Remove(_) => Some(WatchEventType::Delete),
            _ => None,
        };
        let Some(event_type) = event_type else {
            return Vec::new();
        };

        event
            .paths
            .iter()
            .filter(|path| self.is_relevant_path(path))
            .map(|path| (self.rebase_path(path), event_type))
            .collect()
    }

    /// Rebases an event path onto the watched workspace root when the OS
    /// backend reports the canonical location instead (see field doc).
    fn rebase_path(&self, path: &Path) -> PathBuf {
        if path.starts_with(&self.workspace_path) {
            return path.to_path_buf();
        }
        if let Ok(relative) = path.strip_prefix(&self.workspace_canonical) {
            return self.workspace_path.join(relative);
        }
        path.to_path_buf()
    }

    /// Extension + ignored-dirs + gitignore checks (the chokidar `ignored`
    /// option and extension glob in TS).
    fn is_relevant_path(&self, path: &Path) -> bool {
        // Ignore directories-only signals: events for dirs have no supported
        // extension anyway, which the extension check covers.
        let ext = path
            .extension()
            .and_then(|e| e.to_str())
            .map(|e| format!(".{}", e.to_lowercase()));
        let Some(ext) = ext else { return false };
        if !is_supported_extension(&ext) {
            return false;
        }

        // Skip ignored directories at any depth (chokidar **/dir/**)
        if path.components().any(|c| {
            c.as_os_str()
                .to_str()
                .is_some_and(|s| IGNORED_DIRECTORIES.contains(&s))
        }) {
            return false;
        }

        if let Ok(relative) = path.strip_prefix(&self.workspace_path) {
            !self
                .ignore_matcher
                .matched(relative, path.is_dir())
                .is_ignore()
        } else {
            true
        }
    }

    /// Debounced batch processing (TS: processBatch). Deletions first, then
    /// sequential create/change processing with progress callbacks.
    pub async fn process_events(
        &self,
        events: HashMap<PathBuf, WatchEventType>,
        on_file_processed: FileProgressCallback<'_>,
    ) -> BatchSummary {
        let mut summary = BatchSummary::default();

        if events.is_empty() {
            return summary;
        }

        let mut paths_to_delete = Vec::new();
        let mut files_to_process = Vec::new();
        for (file_path, event_type) in events {
            // A create/change whose file no longer exists means it was
            // created and deleted inside one debounce window (macOS FSEvents
            // also emits trailing Modify events after Remove) — the settled
            // end state is a deletion.
            let is_gone = !file_path.exists();
            if is_gone || event_type == WatchEventType::Delete {
                paths_to_delete.push(file_path);
            } else {
                files_to_process.push(file_path);
            }
        }

        // Handle deletions
        if !paths_to_delete.is_empty() {
            if let Some(vector_store) = &self.vector_store {
                let paths: Vec<String> = paths_to_delete
                    .iter()
                    .map(|p| p.to_string_lossy().into_owned())
                    .collect();
                match vector_store
                    .delete_points_by_multiple_file_paths(&paths)
                    .await
                {
                    Ok(()) => {
                        for file_path in paths_to_delete {
                            self.cache_manager.delete_hash(&file_path.to_string_lossy());
                            summary.processed_files.push(FileProcessingResult {
                                path: file_path,
                                status: FileStatus::Success,
                                reason: Some("Deleted".to_string()),
                                new_hash: None,
                            });
                        }
                    }
                    Err(err) => {
                        summary.batch_error = Some(err.to_string());
                        for file_path in paths_to_delete {
                            summary.processed_files.push(FileProcessingResult {
                                path: file_path,
                                status: FileStatus::Error,
                                reason: Some(err.to_string()),
                                new_hash: None,
                            });
                        }
                    }
                }
            }
        }

        // Process creates/changes
        let total = files_to_process.len();
        for (index, path) in files_to_process.into_iter().enumerate() {
            let result = self.process_file(&path).await;
            if let Some(callback) = on_file_processed {
                callback(index + 1, total, &path, &result);
            }
            summary.processed_files.push(result);
        }

        summary
    }

    /// Processes a single file (TS: processFile).
    pub async fn process_file(&self, file_path: &Path) -> FileProcessingResult {
        let fail = |message: String| FileProcessingResult {
            path: file_path.to_path_buf(),
            status: FileStatus::Error,
            reason: Some(message),
            new_hash: None,
        };
        let skip = |reason: &str, new_hash: Option<String>| FileProcessingResult {
            path: file_path.to_path_buf(),
            status: FileStatus::Skipped,
            reason: Some(reason.to_string()),
            new_hash,
        };

        let (Some(embedder), Some(vector_store)) = (&self.embedder, &self.vector_store) else {
            return fail("Embedder or vector store not configured".to_string());
        };

        let result: anyhow::Result<FileProcessingResult> = async {
            let metadata = std::fs::metadata(file_path)?;
            if metadata.len() > MAX_FILE_SIZE_BYTES {
                return Ok(skip("File too large", None));
            }

            let content = std::fs::read_to_string(file_path)?;
            let current_hash = hex::encode(Sha256::digest(content.as_bytes()));

            let path_str = file_path.to_string_lossy().into_owned();
            if self.cache_manager.get_hash(&path_str).as_deref() == Some(current_hash.as_str()) {
                return Ok(skip("Unchanged", None));
            }

            let blocks = self
                .code_parser
                .parse_file(
                    file_path,
                    Some(crate::traits::ParseOptions {
                        content: Some(content),
                        file_hash: Some(current_hash.clone()),
                        ..Default::default()
                    }),
                )
                .await?;

            if blocks.is_empty() {
                self.cache_manager.update_hash(&path_str, current_hash);
                return Ok(skip("No parseable blocks", None));
            }

            // Delete existing points for this file, then upsert (delete-first,
            // consistent with the TS watcher — and with our fixed scanner)
            vector_store.delete_points_by_file_path(&path_str).await?;

            let namespace = Uuid::parse_str(QDRANT_CODE_BLOCK_NAMESPACE)
                .expect("namespace constant is a valid UUID");
            let mut texts = Vec::new();
            let mut kept_blocks = Vec::new();
            for block in &blocks {
                let trimmed = block.content.trim().to_string();
                if !trimmed.is_empty() {
                    texts.push(trimmed);
                    kept_blocks.push(block.clone());
                }
            }

            let embedding_response = embedder.create_embeddings(&texts, None, false).await?;

            let points: Vec<PointStruct> = kept_blocks
                .into_iter()
                .zip(embedding_response.embeddings)
                .map(|(block, vector)| {
                    let mut payload = Map::new();
                    payload.insert("filePath".into(), block.file_path.clone().into());
                    payload.insert("codeChunk".into(), block.content.clone().into());
                    payload.insert("startLine".into(), (block.start_line as u64).into());
                    payload.insert("endLine".into(), (block.end_line as u64).into());
                    payload.insert("segmentHash".into(), block.segment_hash.clone().into());
                    payload.insert("fileHash".into(), block.file_hash.clone().into());
                    PointStruct {
                        id: Uuid::new_v5(&namespace, block.segment_hash.as_bytes()).to_string(),
                        vector,
                        payload,
                    }
                })
                .collect();

            vector_store.upsert_points(points).await?;
            self.cache_manager
                .update_hash(&path_str, current_hash.clone());

            Ok(FileProcessingResult {
                path: file_path.to_path_buf(),
                status: FileStatus::Success,
                reason: None,
                new_hash: Some(current_hash),
            })
        }
        .await;

        result.unwrap_or_else(|err| fail(err.to_string()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::traits::{
        EmbedderInfo, EmbeddingResponse, ValidationResult, VectorStoreSearchResult,
    };
    use std::sync::Mutex;
    use std::time::{SystemTime, UNIX_EPOCH};

    struct MockEmbedder;

    #[async_trait::async_trait]
    impl Embedder for MockEmbedder {
        async fn create_embeddings(
            &self,
            texts: &[String],
            _model: Option<&str>,
            _is_query: bool,
        ) -> anyhow::Result<EmbeddingResponse> {
            Ok(EmbeddingResponse {
                embeddings: texts.iter().map(|_| vec![0.1f32; 4]).collect(),
                usage: None,
            })
        }
        async fn validate_configuration(&self) -> ValidationResult {
            ValidationResult::ok()
        }
        fn embedder_info(&self) -> EmbedderInfo {
            EmbedderInfo {
                name: crate::shared::embedding_models::EmbedderProvider::OpenAi,
            }
        }
    }

    struct MockStore {
        ops: Mutex<Vec<String>>,
    }

    impl MockStore {
        fn new() -> Self {
            Self {
                ops: Mutex::new(Vec::new()),
            }
        }
    }

    #[async_trait::async_trait]
    impl VectorStore for MockStore {
        async fn initialize(&self) -> anyhow::Result<bool> {
            Ok(true)
        }
        async fn upsert_points(&self, points: Vec<PointStruct>) -> anyhow::Result<()> {
            self.ops
                .lock()
                .unwrap()
                .push(format!("upsert:{}", points.len()));
            Ok(())
        }
        async fn search(
            &self,
            _q: &[f32],
            _p: Option<&str>,
            _s: Option<f32>,
            _m: Option<u32>,
        ) -> anyhow::Result<Vec<VectorStoreSearchResult>> {
            Ok(Vec::new())
        }
        async fn delete_points_by_file_path(&self, file_path: &str) -> anyhow::Result<()> {
            self.ops.lock().unwrap().push(format!("delete:{file_path}"));
            Ok(())
        }
        async fn delete_points_by_multiple_file_paths(
            &self,
            file_paths: &[String],
        ) -> anyhow::Result<()> {
            self.ops
                .lock()
                .unwrap()
                .push(format!("delete-many:{}", file_paths.len()));
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

    fn temp_workspace(tag: &str) -> PathBuf {
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let dir = std::env::temp_dir().join(format!("code-index-watcher-test-{tag}-{nanos}"));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn lines(n: usize) -> String {
        (1..=n)
            .map(|i| format!("// line {i} of a reasonably long source file"))
            .collect::<Vec<_>>()
            .join("\n")
    }

    struct Fixture {
        watcher: FileWatcher,
        store: Arc<MockStore>,
        cache: Arc<MockCache>,
    }

    fn fixture(workspace: PathBuf) -> Fixture {
        let store = Arc::new(MockStore::new());
        let cache = Arc::new(MockCache {
            hashes: Mutex::new(HashMap::new()),
        });
        let watcher = FileWatcher::new(
            workspace,
            cache.clone(),
            Some(Arc::new(MockEmbedder)),
            Some(store.clone()),
            ignore::gitignore::Gitignore::empty(),
            None,
        );
        Fixture {
            watcher,
            store,
            cache,
        }
    }

    #[tokio::test]
    async fn process_file_uploads_and_caches() {
        let dir = temp_workspace("process");
        let file = dir.join("a.ts");
        std::fs::write(&file, lines(10)).unwrap();

        let f = fixture(dir.clone());
        let result = f.watcher.process_file(&file).await;
        assert_eq!(result.status, FileStatus::Success);
        assert!(result.new_hash.is_some());
        // delete-first, then upsert
        let ops = f.store.ops.lock().unwrap().clone();
        assert_eq!(ops.len(), 2);
        assert!(ops[0].starts_with("delete:"));
        assert_eq!(ops[1], "upsert:1");
    }

    #[tokio::test]
    async fn process_file_skips_unchanged() {
        let dir = temp_workspace("unchanged");
        let file = dir.join("a.ts");
        std::fs::write(&file, lines(10)).unwrap();

        let f = fixture(dir.clone());
        f.watcher.process_file(&file).await;
        let result = f.watcher.process_file(&file).await;
        assert_eq!(result.status, FileStatus::Skipped);
        assert_eq!(result.reason.as_deref(), Some("Unchanged"));
    }

    #[tokio::test]
    async fn process_file_skips_oversized() {
        let dir = temp_workspace("large");
        let file = dir.join("big.ts");
        std::fs::write(&file, "x".repeat(MAX_FILE_SIZE_BYTES as usize + 1)).unwrap();

        let f = fixture(dir);
        let result = f.watcher.process_file(&file).await;
        assert_eq!(result.status, FileStatus::Skipped);
        assert_eq!(result.reason.as_deref(), Some("File too large"));
    }

    #[tokio::test]
    async fn process_file_skip_empty_updates_cache() {
        let dir = temp_workspace("empty");
        let file = dir.join("empty.ts");
        std::fs::write(&file, "  \n  \n").unwrap();

        let f = fixture(dir);
        let result = f.watcher.process_file(&file).await;
        assert_eq!(result.status, FileStatus::Skipped);
        assert_eq!(result.reason.as_deref(), Some("No parseable blocks"));
        assert!(f.cache.get_hash(&file.to_string_lossy()).is_some());
    }

    #[tokio::test]
    async fn deletions_remove_points_and_cache() {
        let dir = temp_workspace("delete");
        let f = fixture(dir.clone());
        let gone = dir.join("gone.ts");
        f.cache
            .update_hash(&gone.to_string_lossy(), "old-hash".to_string());

        let mut events = HashMap::new();
        events.insert(gone.clone(), WatchEventType::Delete);
        let summary = f.watcher.process_events(events, None).await;

        assert_eq!(summary.success_count(), 1);
        assert!(f.cache.get_hash(&gone.to_string_lossy()).is_none());
        assert_eq!(f.store.ops.lock().unwrap()[0], "delete-many:1");
    }

    #[tokio::test]
    async fn classification_filters_events() {
        let dir = temp_workspace("classify");
        let f = fixture(dir.clone());

        let mk = |kind: EventKind, paths: Vec<PathBuf>| Event {
            kind,
            paths,
            attrs: Default::default(),
        };

        // Supported + not ignored
        let events = f.watcher.classify_event(&mk(
            EventKind::Create(notify::event::CreateKind::File),
            vec![dir.join("a.ts")],
        ));
        assert_eq!(events, vec![(dir.join("a.ts"), WatchEventType::Create)]);

        // Unsupported extension filtered
        assert!(f
            .watcher
            .classify_event(&mk(
                EventKind::Create(notify::event::CreateKind::File),
                vec![dir.join("a.bin")],
            ))
            .is_empty());

        // Ignored directory filtered (node_modules at any depth)
        assert!(f
            .watcher
            .classify_event(&mk(
                EventKind::Modify(ModifyKind::Data(notify::event::DataChange::Any)),
                vec![dir.join("node_modules/pkg/index.ts")],
            ))
            .is_empty());

        // Rename-from maps to delete, rename-to to create
        let from = f.watcher.classify_event(&mk(
            EventKind::Modify(ModifyKind::Name(RenameMode::From)),
            vec![dir.join("old.ts")],
        ));
        assert_eq!(from, vec![(dir.join("old.ts"), WatchEventType::Delete)]);
    }

    #[tokio::test]
    async fn notify_stream_surfaces_events() {
        let dir = temp_workspace("stream");
        let f = fixture(dir.clone());
        let (_watcher, mut rx) = f.watcher.start_notify_stream().unwrap();

        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
        std::fs::write(dir.join("fresh.ts"), lines(5)).unwrap();

        let received = tokio::time::timeout(std::time::Duration::from_secs(5), rx.recv())
            .await
            .expect("event within 5s");
        assert!(received.is_some());
    }
}
