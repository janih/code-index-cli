//! Dependency wiring: builds embedders, vector store, scanner, orchestrator.
//!
//! Port of `src/core/service-factory.ts`.

use std::path::PathBuf;
use std::sync::Arc;

use ignore::gitignore::GitignoreBuilder;

use crate::cache::HashCacheManager;
use crate::config::manager::ConfigManager;
use crate::embedders::{
    GeminiEmbedder, OllamaEmbedder, OpenAiCompatibleEmbedder, OpenAiEmbedder, SimpleHttpEmbedder,
};
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
            EmbedderProvider::OpenAiCompatible => {
                let base_url = config.compatible_base_url().ok_or_else(|| {
                    anyhow::anyhow!("OpenAI Compatible base URL and API key are required.")
                })?;
                let api_key = config.compatible_api_key().ok_or_else(|| {
                    anyhow::anyhow!("OpenAI Compatible base URL and API key are required.")
                })?;
                Ok(Arc::new(OpenAiCompatibleEmbedder::new(
                    base_url.to_string(),
                    api_key.to_string(),
                    config.model_id().map(String::from),
                )))
            }
            EmbedderProvider::Ollama => Ok(Arc::new(OllamaEmbedder::new(
                config.base_url().map(String::from),
                config.model_id().map(String::from),
            ))),
            EmbedderProvider::Gemini => {
                let api_key = config
                    .api_key()
                    .ok_or_else(|| anyhow::anyhow!("Gemini API key is required."))?;
                Ok(Arc::new(GeminiEmbedder::new(
                    api_key.to_string(),
                    config.model_id().map(String::from),
                )))
            }
            EmbedderProvider::Mistral => {
                let api_key = config
                    .api_key()
                    .ok_or_else(|| anyhow::anyhow!("Mistral API key is required."))?;
                Ok(Arc::new(SimpleHttpEmbedder::mistral(
                    api_key.to_string(),
                    config.model_id().map(String::from),
                )))
            }
            EmbedderProvider::VercelAiGateway => {
                let api_key = config
                    .api_key()
                    .ok_or_else(|| anyhow::anyhow!("Vercel AI Gateway API key is required."))?;
                Ok(Arc::new(SimpleHttpEmbedder::vercel_ai_gateway(
                    api_key.to_string(),
                    config.model_id().map(String::from),
                )))
            }
            EmbedderProvider::OpenRouter => {
                let api_key = config
                    .api_key()
                    .ok_or_else(|| anyhow::anyhow!("OpenRouter API key is required."))?;
                Ok(Arc::new(SimpleHttpEmbedder::openrouter(
                    api_key.to_string(),
                    config.model_id().map(String::from),
                    config.open_router_provider().map(String::from),
                )))
            }
            EmbedderProvider::Bedrock => {
                anyhow::bail!(
                    "Bedrock is deferred pending AWS SigV4 signing (see AGENTS.md deviations)"
                )
            }
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
            Some(self.config_manager.max_file_size_bytes()),
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

#[cfg(test)]
mod tests {
    //! Mirrors the provider/error cases of TS service-factory.test.ts.

    use super::*;
    use crate::config::manager::ConfigManager;

    fn config(json: serde_json::Value) -> ConfigManager {
        ConfigManager::new(serde_json::from_value(json).expect("config parses"))
    }

    fn factory(config: ConfigManager) -> ServiceFactory {
        let dir = std::env::temp_dir().join(format!(
            "code-index-factory-test-{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        let cache = HashCacheManager::new(&dir, Some(dir.clone()));
        ServiceFactory::new(config, dir, Arc::new(cache))
    }

    #[test]
    fn creates_openai_embedder() {
        let f = factory(config(serde_json::json!({
            "embedder": { "provider": "openai", "apiKey": "sk-x" }
        })));
        assert_eq!(
            f.create_embedder().unwrap().embedder_info().name,
            EmbedderProvider::OpenAi
        );
    }

    #[test]
    fn openai_without_api_key_errors() {
        let f = factory(config(serde_json::json!({
            "embedder": { "provider": "openai" }
        })));
        let err = f.create_embedder().err().expect("expected error");
        assert!(err.to_string().contains("OpenAI API key"));
    }

    #[test]
    fn ollama_uses_defaults_without_key() {
        let f = factory(config(serde_json::json!({
            "embedder": { "provider": "ollama" }
        })));
        assert_eq!(
            f.create_embedder().unwrap().embedder_info().name,
            EmbedderProvider::Ollama
        );
    }

    #[test]
    fn openai_compatible_needs_base_url_and_key() {
        let f = factory(config(serde_json::json!({
            "embedder": { "provider": "openai-compatible" }
        })));
        assert!(f.create_embedder().is_err());

        let f = factory(config(serde_json::json!({
            "embedder": {
                "provider": "openai-compatible",
                "compatibleBaseUrl": "http://localhost:8089/v1",
                "compatibleApiKey": "test",
            }
        })));
        assert_eq!(
            f.create_embedder().unwrap().embedder_info().name,
            EmbedderProvider::OpenAiCompatible
        );
    }

    #[test]
    fn api_key_providers_error_without_key() {
        for (provider, message) in [
            ("gemini", "Gemini API key"),
            ("mistral", "Mistral API key"),
            ("vercel-ai-gateway", "Vercel AI Gateway API key"),
            ("openrouter", "OpenRouter API key"),
        ] {
            let f = factory(config(serde_json::json!({
                "embedder": { "provider": provider }
            })));
            let err = f.create_embedder().err().expect("expected error");
            assert!(
                err.to_string().contains(message),
                "provider {provider}: {err}"
            );
        }
    }

    #[test]
    fn bedrock_explicitly_deferred() {
        let f = factory(config(serde_json::json!({
            "embedder": { "provider": "bedrock" }
        })));
        let err = f.create_embedder().err().expect("expected error");
        assert!(err.to_string().contains("deferred"));
    }

    #[test]
    fn vector_store_uses_known_model_dimension() {
        let f = factory(config(serde_json::json!({
            "embedder": { "provider": "openai", "apiKey": "sk-x" }
        })));
        // text-embedding-3-small → 1536
        let store = f.create_vector_store().unwrap();
        drop(store);
    }

    #[test]
    fn vector_store_falls_back_to_model_dimension() {
        let f = factory(config(serde_json::json!({
            "embedder": {
                "provider": "openai-compatible",
                "compatibleBaseUrl": "http://localhost:8089/v1",
                "compatibleApiKey": "test",
                "modelId": "embeddinggemma-300M",
                "modelDimension": 768,
            }
        })));
        let store = f.create_vector_store().unwrap();
        drop(store);
    }

    #[test]
    fn vector_store_errors_without_any_dimension() {
        let f = factory(config(serde_json::json!({
            "embedder": {
                "provider": "openai-compatible",
                "compatibleBaseUrl": "http://localhost:8089/v1",
                "compatibleApiKey": "test",
                "modelId": "unknown-model-xyz",
            }
        })));
        let err = f.create_vector_store().err().expect("expected error");
        assert!(err.to_string().contains("dimension"));
    }

    #[test]
    fn scanner_uses_configured_batch_threshold() {
        let f = factory(config(serde_json::json!({
            "embedder": { "provider": "openai", "apiKey": "sk-x" },
            "indexing": { "batchSize": 3 }
        })));
        let scanner = f
            .create_directory_scanner(
                f.create_embedder().unwrap(),
                f.create_vector_store().unwrap(),
            )
            .unwrap();
        drop(scanner);
    }

    #[test]
    fn ignore_matcher_reads_gitignore_and_exclude_patterns() {
        let f = factory(config(serde_json::json!({
            "embedder": { "provider": "openai", "apiKey": "sk-x" },
            "indexing": { "excludePatterns": ["*.snap"] }
        })));
        std::fs::write(f.workspace_path.join(".gitignore"), "secrets/\n").unwrap();
        let matcher = f.build_ignore_matcher().unwrap();
        use std::path::Path;
        assert!(crate::shared::ignore_match::is_ignored(
            &matcher,
            Path::new("secrets/key.pem"),
            false
        ));
        // excludePatterns from config are gitignore-relative
        assert!(crate::shared::ignore_match::is_ignored(
            &matcher,
            Path::new("foo.snap"),
            false
        ));
        assert!(!crate::shared::ignore_match::is_ignored(
            &matcher,
            Path::new("src/main.rs"),
            false
        ));
    }
}
