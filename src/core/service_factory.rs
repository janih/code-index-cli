//! Dependency wiring: builds embedders, vector store, scanner, orchestrator.
//!
//! Port of `src/core/service-factory.ts`.

use std::path::PathBuf;
use std::sync::Arc;

use ignore::gitignore::GitignoreBuilder;

use crate::cache::HashCacheManager;
use crate::config::manager::ConfigManager;
use crate::embedders::OpenAiEmbedder;
use crate::log;
use crate::processors::parser::LineCodeParser;
use crate::processors::scanner::DirectoryScanner;
use crate::shared::constants::BATCH_SEGMENT_THRESHOLD;
use crate::shared::embedding_models::{
    get_default_model_id, get_model_dimension, EmbedderProvider,
};
use crate::traits::{CacheManager, Embedder, ValidationResult, VectorStore};
use crate::vector_store::QdrantVectorStore;

use super::orchestrator::Orchestrator;
use super::state_manager::StateManager;

/// Factory class responsible for creating and configuring code indexing
/// service dependencies.
pub struct ServiceFactory {
    config_manager: ConfigManager,
    workspace_path: PathBuf,
    cache_manager: Arc<HashCacheManager>,
}

impl ServiceFactory {
    pub fn new(
        config_manager: ConfigManager,
        workspace_path: PathBuf,
        cache_manager: Arc<HashCacheManager>,
    ) -> Self {
        Self {
            config_manager,
            workspace_path,
            cache_manager,
        }
    }

    /// Creates an embedder instance based on the current configuration.
    ///
    /// Providers are ported incrementally (Phase 1: openai; Phase 2: the
    /// rest). Unported providers fail loudly here rather than silently
    /// misbehaving.
    pub fn create_embedder(&self) -> anyhow::Result<Arc<dyn Embedder>> {
        let config = &self.config_manager;
        let provider = config.embedder_provider();

        match provider {
            EmbedderProvider::OpenAi => {
                let api_key = config.api_key().ok_or_else(|| {
					anyhow::anyhow!(
						"OpenAI API key is required. Set CODE_INDEX_CLI_EMBEDDER_API_KEY or configure in .code-index.json"
					)
				})?;
                Ok(Arc::new(OpenAiEmbedder::new(
                    api_key.to_string(),
                    config.model_id().map(String::from),
                )))
            }
            other => anyhow::bail!(
                "Embedder provider '{}' is not ported to the Rust version yet (Phase 2).",
                other
            ),
        }
    }

    /// Validates an embedder instance.
    pub async fn validate_embedder(&self, embedder: &dyn Embedder) -> ValidationResult {
        embedder.validate_configuration().await
    }

    /// Creates a vector store instance.
    pub fn create_vector_store(&self) -> anyhow::Result<Arc<dyn VectorStore>> {
        let config = &self.config_manager;
        let provider = config.embedder_provider();
        let model_id = config
            .model_id()
            .unwrap_or_else(|| get_default_model_id(provider));

        let vector_size = match get_model_dimension(provider, model_id) {
            Some(dimension) => dimension,
            None => match config.model_dimension() {
                Some(dimension) if dimension > 0 => dimension as usize,
                _ => anyhow::bail!(
					"Could not determine vector dimension for model \"{model_id}\" (provider: {provider}). \
					 Set embedder.modelDimension in your config file."
				),
            },
        };

        Ok(Arc::new(QdrantVectorStore::new(
            &self.workspace_path.to_string_lossy(),
            Some(config.qdrant_url()),
            vector_size,
            config.qdrant_api_key(),
        )))
    }

    /// Creates a directory scanner instance.
    pub fn create_directory_scanner(
        &self,
        embedder: Arc<dyn Embedder>,
        vector_store: Arc<dyn VectorStore>,
    ) -> anyhow::Result<Arc<DirectoryScanner>> {
        let batch_size = self.config_manager.batch_size();
        let threshold = if batch_size > 0 {
            Some(batch_size as usize)
        } else {
            Some(BATCH_SEGMENT_THRESHOLD)
        };
        Ok(Arc::new(DirectoryScanner::new(
            embedder,
            vector_store,
            Arc::new(LineCodeParser::new()),
            Arc::clone(&self.cache_manager) as Arc<dyn CacheManager>,
            self.build_ignore_matcher()?,
            threshold,
        )))
    }

    /// Builds a gitignore matcher from the workspace `.gitignore` and the
    /// configured exclude patterns (TS `buildIgnoreInstance`).
    pub fn build_ignore_matcher(&self) -> anyhow::Result<ignore::gitignore::Gitignore> {
        let mut builder = GitignoreBuilder::new(&self.workspace_path);

        let gitignore_path = self.workspace_path.join(".gitignore");
        if gitignore_path.exists() {
            builder.add(&gitignore_path);
            log::debug(&format!(
                "Loaded .gitignore from {}",
                gitignore_path.display()
            ));
        }

        for pattern in self.config_manager.exclude_patterns() {
            if let Err(err) = builder.add_line(None, pattern) {
                log::debug(&format!(
                    "Skipping invalid exclude pattern {pattern:?}: {err}"
                ));
            }
        }
        let exclude_count = self.config_manager.exclude_patterns().len();
        if exclude_count > 0 {
            log::debug(&format!(
                "Added {exclude_count} exclude patterns from config"
            ));
        }

        Ok(builder.build()?)
    }

    /// Creates a fully wired orchestrator (no file watcher — watch mode is
    /// composed by the watch command itself in Phase 3).
    pub fn create_orchestrator(
        &self,
        state_manager: Arc<StateManager>,
    ) -> anyhow::Result<Orchestrator> {
        let embedder = self.create_embedder()?;
        let vector_store = self.create_vector_store()?;
        let scanner =
            self.create_directory_scanner(Arc::clone(&embedder), Arc::clone(&vector_store))?;

        Ok(Orchestrator::new(
            self.config_manager.clone(),
            state_manager,
            self.workspace_path.clone(),
            Arc::clone(&self.cache_manager),
            vector_store,
            scanner,
        ))
    }
}
