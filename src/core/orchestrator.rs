//! Indexing workflow orchestrator.
//!
//! Port of `src/core/orchestrator.ts`. The file watcher is deliberately not
//! owned here (watch mode composes it in Phase 3); `start_indexing` performs
//! the scan and returns the outcome.

use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

use crate::cache::HashCacheManager;
use crate::config::manager::ConfigManager;
use crate::log;
use crate::processors::scanner::{DirectoryScanner, ScanCallbacks, ScanOutcome};
use crate::shared::cancellation::CancellationToken;
use crate::traits::{CacheManager, VectorStore};

use super::state_manager::{IndexingState, StateManager};

/// Manages the code indexing workflow, coordinating between services.
pub struct Orchestrator {
    config_manager: ConfigManager,
    state_manager: Arc<StateManager>,
    workspace_path: PathBuf,
    cache_manager: Arc<HashCacheManager>,
    vector_store: Arc<dyn VectorStore>,
    scanner: Arc<DirectoryScanner>,
    is_processing: AtomicBool,
    cancel_token: std::sync::Mutex<Option<CancellationToken>>,
}

impl Orchestrator {
    pub fn new(
        config_manager: ConfigManager,
        state_manager: Arc<StateManager>,
        workspace_path: PathBuf,
        cache_manager: Arc<HashCacheManager>,
        vector_store: Arc<dyn VectorStore>,
        scanner: Arc<DirectoryScanner>,
    ) -> Self {
        Self {
            config_manager,
            state_manager,
            workspace_path,
            cache_manager,
            vector_store,
            scanner,
            is_processing: AtomicBool::new(false),
            cancel_token: Mutex::new(None),
        }
    }

    pub fn state(&self) -> IndexingState {
        self.state_manager.state()
    }

    /// Shared scan logic used by both full and incremental indexing paths.
    async fn run_scan(
        &self,
        token: &CancellationToken,
        scan_label: &str,
    ) -> anyhow::Result<ScanOutcome> {
        let cumulative_blocks_indexed = Arc::new(AtomicUsize::new(0));
        let cumulative_blocks_found = Arc::new(AtomicUsize::new(0));
        let batch_errors: Arc<Mutex<Vec<String>>> = Arc::new(Mutex::new(Vec::new()));

        let state_manager = Arc::clone(&self.state_manager);
        let on_file_parsed = {
            let found = Arc::clone(&cumulative_blocks_found);
            let indexed = Arc::clone(&cumulative_blocks_indexed);
            let sm = Arc::clone(&state_manager);
            move |file_block_count: usize| {
                let f = found.fetch_add(file_block_count, Ordering::SeqCst) + file_block_count;
                let i = indexed.load(Ordering::SeqCst);
                sm.report_block_indexing_progress(i, f);
            }
        };

        let on_blocks_indexed = {
            let found = Arc::clone(&cumulative_blocks_found);
            let indexed = Arc::clone(&cumulative_blocks_indexed);
            let sm = Arc::clone(&state_manager);
            move |indexed_count: usize| {
                let i = indexed.fetch_add(indexed_count, Ordering::SeqCst) + indexed_count;
                let f = found.load(Ordering::SeqCst);
                sm.report_block_indexing_progress(i, f);
            }
        };

        let on_error = {
            let errors = Arc::clone(&batch_errors);
            let label = scan_label.to_string();
            move |err: anyhow::Error| {
                log::error(&format!("Error during {label}: {err}"));
                errors
                    .lock()
                    .unwrap_or_else(|e| e.into_inner())
                    .push(err.to_string());
            }
        };

        let result = self
            .scanner
            .scan_directory(
                &self.workspace_path,
                ScanCallbacks {
                    on_error: Some(Arc::new(on_error)),
                    on_blocks_indexed: Some(Arc::new(on_blocks_indexed)),
                    on_file_parsed: Some(Arc::new(on_file_parsed)),
                },
                Some(token.clone()),
            )
            .await?;

        if token.is_cancelled() {
            self.cache_manager.flush().await.ok();
            self.state_manager
                .set_system_state(IndexingState::Standby, Some("Indexing stopped."));
            return Ok(result);
        }

        let indexed = cumulative_blocks_indexed.load(Ordering::SeqCst);
        let found = cumulative_blocks_found.load(Ordering::SeqCst);
        let errors = batch_errors.lock().unwrap_or_else(|e| e.into_inner());

        if indexed == 0 && found > 0 && !errors.is_empty() {
            anyhow::bail!("Indexing failed: {}", errors[0]);
        }

        if found > 0 {
            log::info(&format!(
                "{scan_label}: {indexed} blocks indexed from new/changed files"
            ));
        } else {
            log::info("No new or changed files found");
        }

        Ok(result)
    }

    /// Initiates the indexing process (initial or incremental scan).
    pub async fn start_indexing(&self) -> anyhow::Result<()> {
        if !self.config_manager.is_feature_configured() {
            self.state_manager
                .set_system_state(IndexingState::Standby, Some("Missing configuration."));
            log::warn("Start rejected: Missing configuration.");
            return Ok(());
        }

        let current = self.state_manager.state();
        if self.is_processing.load(Ordering::SeqCst)
            || !matches!(
                current,
                IndexingState::Standby | IndexingState::Error | IndexingState::Indexed
            )
        {
            log::warn(&format!(
                "Start rejected: Already processing or in state {current}."
            ));
            return Ok(());
        }

        self.is_processing.store(true, Ordering::SeqCst);
        let token = CancellationToken::new();
        *self.cancel_token.lock().unwrap_or_else(|e| e.into_inner()) = Some(token.clone());
        self.state_manager
            .set_system_state(IndexingState::Indexing, Some("Initializing services..."));

        let result = self.run_indexing_flow(&token).await;

        // Error path (TS catch): clean up on failure unless aborted
        if let Err(err) = &result {
            if token.is_cancelled() {
                log::info("Indexing aborted by user.");
                self.cache_manager.flush().await.ok();
                self.state_manager
                    .set_system_state(IndexingState::Standby, Some("Indexing stopped."));
            } else {
                log::error(&format!("Error during indexing: {err}"));
                // TS failure path: if initialize() succeeded, wipe partial data
                if self.vector_store.collection_exists().await.unwrap_or(false) {
                    if let Err(cleanup_err) = self.vector_store.clear_collection().await {
                        log::error(&format!("Failed to clean up after error: {cleanup_err}"));
                    }
                    self.cache_manager.clear_cache_file();
                }
                self.state_manager.set_system_state(
                    IndexingState::Error,
                    Some(&format!("Indexing failed: {err}")),
                );
            }
        }

        self.is_processing.store(false, Ordering::SeqCst);
        *self.cancel_token.lock().unwrap_or_else(|e| e.into_inner()) = None;

        result
    }

    async fn run_indexing_flow(&self, token: &CancellationToken) -> anyhow::Result<()> {
        let collection_created = self.vector_store.initialize().await?;

        if collection_created {
            self.cache_manager.clear_cache_file();
        }

        let has_existing_data = self.vector_store.has_indexed_data().await?;

        if has_existing_data && !collection_created {
            log::info("Collection has existing data. Running incremental scan...");
            self.state_manager.set_system_state(
                IndexingState::Indexing,
                Some("Checking for new or modified files..."),
            );
            self.vector_store.mark_indexing_incomplete().await?;

            self.run_scan(token, "incremental scan").await?;
            if token.is_cancelled() {
                return Ok(());
            }

            self.vector_store.mark_indexing_complete().await?;
        } else {
            self.state_manager
                .set_system_state(IndexingState::Indexing, Some("Starting workspace scan..."));
            self.vector_store.mark_indexing_incomplete().await?;

            let outcome = self.run_scan(token, "initial scan").await?;
            if token.is_cancelled() {
                return Ok(());
            }

            self.vector_store.mark_indexing_complete().await?;
            log::info(&format!(
                "Indexing complete: {} files processed, {} skipped, {} blocks indexed",
                outcome.stats.processed, outcome.stats.skipped, outcome.total_block_count
            ));
        }

        self.state_manager
            .set_system_state(IndexingState::Indexed, Some("Index up-to-date."));
        Ok(())
    }

    /// Stops any in-progress indexing.
    pub fn stop_indexing(&self) {
        let token = self
            .cancel_token
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .take();
        if let Some(token) = token {
            self.state_manager
                .set_system_state(IndexingState::Stopping, Some("Stopping indexing..."));
            token.cancel();
        }
    }

    /// Clears all index data.
    pub async fn clear_index_data(&self) -> anyhow::Result<()> {
        self.is_processing.store(true, Ordering::SeqCst);

        let () = async {
            if self.config_manager.is_feature_configured() {
                if let Err(err) = self.vector_store.delete_collection().await {
                    log::error(&format!("Failed to clear vector collection: {err}"));
                    self.state_manager.set_system_state(
                        IndexingState::Error,
                        Some(&format!("Failed to clear: {err}")),
                    );
                }
            }

            self.cache_manager.clear_cache_file();

            if self.state_manager.state() != IndexingState::Error {
                self.state_manager.set_system_state(
                    IndexingState::Standby,
                    Some("Index data cleared successfully."),
                );
            }
        }
        .await;

        self.is_processing.store(false, Ordering::SeqCst);
        Ok(())
    }
}
