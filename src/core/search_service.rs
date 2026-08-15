//! Semantic search service.
//!
//! Port of `src/core/search-service.ts`: embed the query (query-prefix
//! aware), vector search, then exact directory-prefix post-filtering.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use crate::config::manager::ConfigManager;
use crate::log;
use crate::traits::{Embedder, VectorStore, VectorStoreSearchResult};

use super::state_manager::{IndexingState, StateManager};

/// Service responsible for searching the code index.
pub struct SearchService {
    config_manager: ConfigManager,
    state_manager: Arc<StateManager>,
    workspace_path: PathBuf,
    embedder: Arc<dyn Embedder>,
    vector_store: Arc<dyn VectorStore>,
}

impl SearchService {
    pub fn new(
        config_manager: ConfigManager,
        state_manager: Arc<StateManager>,
        workspace_path: PathBuf,
        embedder: Arc<dyn Embedder>,
        vector_store: Arc<dyn VectorStore>,
    ) -> Self {
        Self {
            config_manager,
            state_manager,
            workspace_path,
            embedder,
            vector_store,
        }
    }

    /// Searches the code index for relevant content.
    pub async fn search_index(
        &self,
        query: &str,
        directory_prefix: Option<&str>,
        max_results: Option<u32>,
    ) -> anyhow::Result<Vec<VectorStoreSearchResult>> {
        if !self.config_manager.is_feature_configured() {
            anyhow::bail!("Code index feature is not configured.");
        }

        let effective_min_score = self.config_manager.effective_search_min_score();
        let effective_max_results =
            max_results.unwrap_or_else(|| self.config_manager.search_max_results());

        let current_state = self.state_manager.state();
        if !matches!(
            current_state,
            IndexingState::Indexed | IndexingState::Indexing
        ) {
            anyhow::bail!("Code index is not ready for search. Current state: {current_state}");
        }

        let result = self
            .run_search(
                query,
                directory_prefix,
                effective_min_score as f32,
                effective_max_results,
            )
            .await;

        if let Err(err) = &result {
            log::error(&format!("Error during search: {err}"));
            self.state_manager
                .set_system_state(IndexingState::Error, Some(&format!("Search failed: {err}")));
        }
        result
    }

    async fn run_search(
        &self,
        query: &str,
        directory_prefix: Option<&str>,
        min_score: f32,
        max_results: u32,
    ) -> anyhow::Result<Vec<VectorStoreSearchResult>> {
        // is_query=true applies model-specific query prefixes
        let embedding_response = self
            .embedder
            .create_embeddings(&[query.to_string()], None, true)
            .await?;
        let vector = embedding_response
            .embeddings
            .into_iter()
            .next()
            .ok_or_else(|| anyhow::anyhow!("Failed to generate embedding for query."))?;

        let normalized_prefix =
            directory_prefix.map(|prefix| normalize_directory_prefix(prefix, &self.workspace_path));

        let results = self
            .vector_store
            .search(
                &vector,
                normalized_prefix.as_deref(),
                Some(min_score),
                Some(max_results),
            )
            .await?;

        let results = match normalized_prefix {
            Some(prefix) => results
                .into_iter()
                .filter(|r| {
                    r.payload
                        .as_ref()
                        .is_some_and(|p| p.file_path.starts_with(prefix.as_str()))
                })
                .take(max_results as usize)
                .collect(),
            None => results.into_iter().take(max_results as usize).collect(),
        };

        Ok(results)
    }
}

/// Resolves a (possibly relative) prefix against the workspace and ensures
/// a trailing separator, matching the stored file paths.
///
/// Relative prefixes resolve against the workspace string given on the CLI —
/// NOT `current_dir()`: on macOS the CWD canonicalizes symlinks (e.g.
/// /tmp -> /private/tmp) while stored paths keep the workspace string as
/// given, so CWD-based resolution silently matched nothing (found during
/// live verification of `--directory`, review round 1). Lexical
/// normalization only (collapses `.` / `..`), like Node's path.resolve.
fn normalize_directory_prefix(directory_prefix: &str, base: &Path) -> String {
    let path = Path::new(directory_prefix);
    let resolved = if path.is_absolute() {
        path.to_path_buf()
    } else {
        base.join(path)
    };
    // Lexical normalization like Node's path.resolve (collapses "." / ".."):
    let mut parts: Vec<std::path::Component> = Vec::new();
    for component in resolved.components() {
        match component {
            std::path::Component::CurDir => {}
            std::path::Component::ParentDir => {
                if matches!(parts.last(), Some(std::path::Component::Normal(_))) {
                    parts.pop();
                } else {
                    parts.push(component);
                }
            }
            other => parts.push(other),
        }
    }
    let normalized_path: PathBuf = parts.iter().collect();
    let mut normalized = normalized_path.to_string_lossy().replace('\\', "/");
    if !normalized.ends_with('/') {
        normalized.push('/');
    }
    normalized
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::state_manager::IndexingState;
    use crate::traits::{EmbedderInfo, EmbeddingResponse, Payload, ValidationResult};
    use std::collections::HashMap;
    use std::sync::Mutex;

    use crate::shared::embedding_models::EmbedderProvider;

    struct MockEmbedder {
        last_is_query: Mutex<Option<bool>>,
    }

    #[async_trait::async_trait]
    impl Embedder for MockEmbedder {
        async fn create_embeddings(
            &self,
            texts: &[String],
            _model: Option<&str>,
            is_query: bool,
        ) -> anyhow::Result<EmbeddingResponse> {
            *self.last_is_query.lock().unwrap() = Some(is_query);
            Ok(EmbeddingResponse {
                embeddings: texts.iter().map(|_| vec![0.5f32; 8]).collect(),
                usage: None,
            })
        }
        async fn validate_configuration(&self) -> ValidationResult {
            ValidationResult::ok()
        }
        fn embedder_info(&self) -> EmbedderInfo {
            EmbedderInfo {
                name: EmbedderProvider::OpenAi,
            }
        }
    }

    struct MockStore {
        results: Vec<VectorStoreSearchResult>,
        last_prefix: Mutex<Option<String>>,
    }

    #[async_trait::async_trait]
    impl VectorStore for MockStore {
        async fn initialize(&self) -> anyhow::Result<bool> {
            Ok(false)
        }
        async fn upsert_points(
            &self,
            _points: Vec<crate::traits::PointStruct>,
        ) -> anyhow::Result<()> {
            Ok(())
        }
        async fn search(
            &self,
            _query: &[f32],
            prefix: Option<&str>,
            _min_score: Option<f32>,
            _max: Option<u32>,
        ) -> anyhow::Result<Vec<VectorStoreSearchResult>> {
            *self.last_prefix.lock().unwrap() = prefix.map(String::from);
            Ok(self.results.clone())
        }
        async fn delete_points_by_file_path(&self, _p: &str) -> anyhow::Result<()> {
            Ok(())
        }
        async fn delete_points_by_multiple_file_paths(&self, _p: &[String]) -> anyhow::Result<()> {
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

    fn payload(file_path: &str) -> Payload {
        Payload {
            file_path: file_path.to_string(),
            code_chunk: "chunk".into(),
            start_line: 1,
            end_line: 5,
            extra: HashMap::new(),
        }
    }

    /// Minimum viable configured config for the search service.
    fn test_config() -> ConfigManager {
        let mut json = serde_json::json!({});
        json["embedder"] = serde_json::json!({
            "provider": "openai",
            "apiKey": "sk-test",
            "modelId": "text-embedding-3-small",
        });
        let config: crate::config::schema::CliConfig =
            serde_json::from_value(json).expect("config parses");
        ConfigManager::new(config)
    }

    fn service_with(
        results: Vec<VectorStoreSearchResult>,
        state: IndexingState,
    ) -> (SearchService, Arc<MockEmbedder>, Arc<MockStore>) {
        let embedder = Arc::new(MockEmbedder {
            last_is_query: Mutex::new(None),
        });
        let store = Arc::new(MockStore {
            results,
            last_prefix: Mutex::new(None),
        });
        let sm = Arc::new(StateManager::new());
        sm.set_system_state(state, None);
        (
            SearchService::new(
                test_config(),
                sm,
                PathBuf::from("/ws"),
                embedder.clone(),
                store.clone(),
            ),
            embedder,
            store,
        )
    }

    fn result_with_path(path: &str) -> VectorStoreSearchResult {
        VectorStoreSearchResult {
            id: serde_json::json!("id"),
            score: 0.9,
            payload: Some(payload(path)),
        }
    }

    #[tokio::test]
    async fn search_uses_query_prefix_and_returns_results() {
        let (service, embedder, _store) = service_with(
            vec![result_with_path("/ws/src/a.ts")],
            IndexingState::Indexed,
        );
        let results = service
            .search_index("entry point", None, None)
            .await
            .unwrap();
        assert_eq!(results.len(), 1);
        assert_eq!(*embedder.last_is_query.lock().unwrap(), Some(true));
    }

    #[tokio::test]
    async fn rejects_search_when_not_indexed() {
        let (service, _e, _s) = service_with(vec![], IndexingState::Standby);
        let err = service.search_index("q", None, None).await.unwrap_err();
        assert!(err.to_string().contains("not ready"));
    }

    #[tokio::test]
    async fn directory_prefix_is_normalized_and_post_filtered() {
        let results = vec![
            result_with_path("/ws/src/a.ts"),
            result_with_path("/ws/other/b.ts"),
        ];
        let (service, _e, store) = service_with(results, IndexingState::Indexed);

        // Relative prefix resolves against the WORKSPACE (not CWD — CWD
        // canonicalization broke matching on macOS symlinked paths).
        let filtered = service.search_index("q", Some("src"), None).await.unwrap();
        assert_eq!(filtered.len(), 1);
        assert_eq!(
            filtered[0].payload.as_ref().unwrap().file_path,
            "/ws/src/a.ts"
        );
        let sent_prefix = store.last_prefix.lock().unwrap().clone().unwrap();
        assert_eq!(sent_prefix, "/ws/src/");
    }

    #[tokio::test]
    async fn directory_prefix_supports_dot_and_dotdot() {
        let results = vec![
            result_with_path("/ws/src/a.ts"),
            result_with_path("/ws/other/b.ts"),
        ];
        let (service, _e, store) = service_with(results, IndexingState::Indexed);

        let filtered = service
            .search_index("q", Some("./src/../src"), None)
            .await
            .unwrap();
        assert_eq!(filtered.len(), 1);
        assert_eq!(sent(store.clone()), "/ws/src/");

        let filtered = service.search_index("q", Some("."), None).await.unwrap();
        assert_eq!(filtered.len(), 2);
        assert_eq!(sent(store), "/ws/");
    }

    fn sent(store: Arc<MockStore>) -> String {
        store.last_prefix.lock().unwrap().clone().unwrap()
    }

    #[tokio::test]
    async fn directory_prefix_matches_stored_absolute_paths() {
        let results = vec![
            result_with_path("/ws/src/a.ts"),
            result_with_path("/ws/other/b.ts"),
        ];
        let (service, _e, _store) = service_with(results, IndexingState::Indexed);
        let filtered = service
            .search_index("q", Some("/ws/src"), None)
            .await
            .unwrap();
        assert_eq!(filtered.len(), 1);
        assert_eq!(
            filtered[0].payload.as_ref().unwrap().file_path,
            "/ws/src/a.ts"
        );
    }
}
