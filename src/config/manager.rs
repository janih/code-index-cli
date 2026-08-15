//! Typed accessor over the resolved configuration.
//!
//! Port of `src/config/config-manager.ts`.

use crate::shared::embedding_models::{get_model_score_threshold, EmbedderProvider};

use super::schema::CliConfig;

/// Manages configuration state and provides typed access to values.
#[derive(Clone)]
pub struct ConfigManager {
    config: CliConfig,
}

impl ConfigManager {
    pub fn new(config: CliConfig) -> Self {
        Self { config }
    }

    /// Full configuration object.
    pub fn config(&self) -> &CliConfig {
        &self.config
    }

    /// Whether the current provider has its required fields configured.
    pub fn is_feature_configured(&self) -> bool {
        match self.config.embedder.provider {
            EmbedderProvider::OpenAi
            | EmbedderProvider::Gemini
            | EmbedderProvider::Mistral
            | EmbedderProvider::VercelAiGateway
            | EmbedderProvider::OpenRouter => self.config.embedder.api_key.is_some(),
            EmbedderProvider::Ollama => true, // defaults to localhost:11434
            EmbedderProvider::OpenAiCompatible => {
                self.config.embedder.compatible_base_url.is_some()
                    && self.config.embedder.compatible_api_key.is_some()
            }
            EmbedderProvider::Bedrock => !self.config.embedder.bedrock_region.is_empty(),
        }
    }

    pub fn embedder_provider(&self) -> EmbedderProvider {
        self.config.embedder.provider
    }

    pub fn model_id(&self) -> Option<&str> {
        self.config.embedder.model_id.as_deref()
    }

    pub fn model_dimension(&self) -> Option<u32> {
        self.config.embedder.model_dimension
    }

    pub fn qdrant_url(&self) -> &str {
        &self.config.qdrant.url
    }

    pub fn qdrant_api_key(&self) -> Option<&str> {
        self.config.qdrant.api_key.as_deref()
    }

    pub fn search_min_score(&self) -> f64 {
        self.config.search.min_score
    }

    /// Effective minimum search score: model-specific threshold if known,
    /// otherwise the configured `search.minScore`.
    pub fn effective_search_min_score(&self) -> f64 {
        get_model_score_threshold(self.embedder_provider(), self.model_id().unwrap_or(""))
            .unwrap_or_else(|| self.search_min_score())
    }

    pub fn search_max_results(&self) -> u32 {
        self.config.search.max_results
    }

    pub fn batch_size(&self) -> u32 {
        self.config.indexing.batch_size
    }

    pub fn max_file_size_bytes(&self) -> u64 {
        self.config.indexing.max_file_size_bytes
    }

    pub fn exclude_patterns(&self) -> &[String] {
        &self.config.indexing.exclude_patterns
    }

    pub fn include_extensions(&self) -> Option<&[String]> {
        self.config.indexing.include_extensions.as_deref()
    }

    pub fn api_key(&self) -> Option<&str> {
        self.config.embedder.api_key.as_deref()
    }

    pub fn base_url(&self) -> Option<&str> {
        self.config.embedder.base_url.as_deref()
    }

    pub fn compatible_base_url(&self) -> Option<&str> {
        self.config.embedder.compatible_base_url.as_deref()
    }

    pub fn compatible_api_key(&self) -> Option<&str> {
        self.config.embedder.compatible_api_key.as_deref()
    }

    pub fn bedrock_region(&self) -> &str {
        if self.config.embedder.bedrock_region.is_empty() {
            "us-east-1"
        } else {
            &self.config.embedder.bedrock_region
        }
    }

    pub fn bedrock_profile(&self) -> Option<&str> {
        self.config.embedder.bedrock_profile.as_deref()
    }

    pub fn open_router_provider(&self) -> Option<&str> {
        self.config.embedder.open_router_provider.as_deref()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::schema::CliConfig;

    fn manager_with(json: &str) -> ConfigManager {
        let config: CliConfig = serde_json::from_str(json).expect("test config parses");
        ConfigManager::new(config)
    }

    #[test]
    fn feature_configured_per_provider() {
        assert!(!ConfigManager::new(CliConfig::default()).is_feature_configured()); // openai without key
        assert!(
            manager_with(r#"{"embedder": {"provider": "openai", "apiKey": "sk-x"}}"#)
                .is_feature_configured()
        );
        assert!(manager_with(r#"{"embedder": {"provider": "ollama"}}"#).is_feature_configured());
        assert!(
            !manager_with(r#"{"embedder": {"provider": "openai-compatible"}}"#)
                .is_feature_configured()
        );
        assert!(manager_with(
			r#"{"embedder": {"provider": "openai-compatible", "compatibleBaseUrl": "http://x", "compatibleApiKey": "k"}}"#
		)
		.is_feature_configured());
        assert!(manager_with(r#"{"embedder": {"provider": "bedrock"}}"#).is_feature_configured());
    }

    #[test]
    fn effective_min_score_prefers_model_threshold() {
        // nomic-embed-code has a model-specific threshold of 0.15
        let manager =
            manager_with(r#"{"embedder": {"provider": "ollama", "modelId": "nomic-embed-code"}}"#);
        assert_eq!(manager.effective_search_min_score(), 0.15);
    }

    #[test]
    fn effective_min_score_falls_back_to_config() {
        let manager = manager_with(
            r#"{"embedder": {"provider": "openai", "modelId": "unknown-model"}, "search": {"minScore": 0.7}}"#,
        );
        assert_eq!(manager.effective_search_min_score(), 0.7);
    }

    #[test]
    fn bedrock_region_falls_back_when_empty() {
        let manager = manager_with(r#"{"embedder": {"bedrockRegion": ""}}"#);
        assert_eq!(manager.bedrock_region(), "us-east-1");
    }

    #[test]
    fn typed_getters() {
        let manager = manager_with(
            r#"{
				"embedder": {"provider": "openai", "modelId": "text-embedding-3-large", "apiKey": "sk-x", "baseUrl": "http://proxy"},
				"qdrant": {"url": "http://qdrant:6333", "apiKey": "qk"},
				"search": {"minScore": 0.55, "maxResults": 75},
				"indexing": {"batchSize": 32, "maxFileSizeBytes": 2048, "excludePatterns": ["dist"], "includeExtensions": [".rs"]}
			}"#,
        );
        assert_eq!(manager.embedder_provider(), EmbedderProvider::OpenAi);
        assert_eq!(manager.model_id(), Some("text-embedding-3-large"));
        assert_eq!(manager.api_key(), Some("sk-x"));
        assert_eq!(manager.base_url(), Some("http://proxy"));
        assert_eq!(manager.qdrant_url(), "http://qdrant:6333");
        assert_eq!(manager.qdrant_api_key(), Some("qk"));
        assert_eq!(manager.search_min_score(), 0.55);
        assert_eq!(manager.search_max_results(), 75);
        assert_eq!(manager.batch_size(), 32);
        assert_eq!(manager.max_file_size_bytes(), 2048);
        assert_eq!(manager.exclude_patterns(), &["dist".to_string()]);
        assert_eq!(manager.include_extensions(), Some(&[".rs".to_string()][..]));
    }
}
